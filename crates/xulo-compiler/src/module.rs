//! Multi-file module loading (spec `modules/`).
//!
//! The loader grows a package's module tree from the entry file: every `mod`
//! declaration claims exactly one file under its module's home directory
//! (spec `modules/source-files.md`), faces are extracted syntactically and
//! completed by a `pub use` fixpoint so `use` cycles are ordinary, and
//! analysis runs in structural initialization order — children before
//! parents, siblings in declaration order, declared dependencies before the
//! package's own tree. Manifest dependencies (`xulo.toml`) load as their own
//! packages rooted at `@path_name`; a version-only dependency gets an
//! external face its members resolve against opaquely.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use xulo_core::ast::{ExportItem, PathHead, Program, Statement, UseDecl, UseTarget};
use xulo_core::error::{ErrorKind, XuloError};
use xulo_runtime::macro_system::{MacroDef, collect_macros, expand_program_with_imports};
use xulo_semantic::{
    AnalysisResult, FaceKind, FaceName, ModuleFace, ModuleWorld, UseBinding, analyze_module_with,
    child_module_id, module_face,
};

/// One loaded module, in structural initialization order (post-order of the
/// module tree: children before parents, siblings in declaration order;
/// dependency packages first).
pub struct Module {
    /// The file whose statements form this module — the parent's file for an
    /// inline `mod { … }`, whose spans live there.
    pub file: PathBuf,
    /// The file's source text (spans in `program` index into it; an inline
    /// `mod { … }` shares its parent file's text).
    pub source: String,
    /// Module id: `""` for the package root, `form` for a root child,
    /// `form::fields` below it; a dependency package root is `@path_name`.
    pub id: String,
    pub program: Program,
    /// Phase-2 analysis result (filled by [`analyze`]).
    pub analysis: Option<AnalysisResult>,
    /// Resolved `use` bindings from analysis, for the runtime to wire each
    /// module's environment (filled by [`analyze`]).
    pub uses: Vec<UseBinding>,
    /// This module's `mod` declarations: (local name, child module index),
    /// so the runtime binds each child's namespace under the name.
    pub mods: Vec<(String, usize)>,
    /// Whether this module declares a `main` function (the entry runs it).
    pub has_main: bool,
}

/// The result of loading an entry file and its transitive `mod` tree.
pub struct LoadedModules {
    /// Initialization order: `modules[j]` initializes before `modules[i]`
    /// iff `j < i` (the world's `init_order` maps each id to its index).
    pub modules: Vec<Module>,
    /// Index of the entry (package root) module: its `main` runs.
    pub entry: usize,
    /// Faces, manifest path names, and initialization order, as the checker
    /// and the runtime see them.
    pub world: ModuleWorld,
}

impl LoadedModules {
    /// Index of the module with this id (dependency packages included);
    /// `None` for a manifest dependency with no local sources.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.modules.iter().position(|m| m.id == id)
    }
}

/// Load, analyze, and annotate the package reachable from `entry`.
/// Returns the non-fatal warnings raised during analysis.
pub fn compile_file(entry: &Path) -> Result<Vec<XuloError>, XuloError> {
    let mut loaded = load(entry)?;
    let warnings = analyze(&mut loaded, true)?;
    apply_trait_dispatch(&mut loaded);
    Ok(warnings)
}

/// Load the entry package's module tree: declared manifest dependencies
/// first (each its own package, post-order), then the package grown from
/// `entry` itself. Faces are extracted and the `pub use` fixpoint completes
/// re-exports; phase-2 analysis runs in [`analyze`].
pub fn load(entry: &Path) -> Result<LoadedModules, XuloError> {
    let entry = entry.canonicalize().map_err(|e| {
        XuloError::new(
            ErrorKind::Io,
            format!("cannot resolve {}: {e}", entry.display()),
        )
    })?;
    let package_dir = entry
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let deps = parse_manifest(&package_dir)?;
    let mut loader = Loader {
        claimed: HashMap::new(),
        modules: Vec::new(),
        id_to_idx: HashMap::new(),
        packages: HashMap::new(),
        external_roots: Vec::new(),
    };
    // Declared dependencies initialize before this package's own tree
    // (spec `modules/import-export.md`, "Initialization").
    for dep in &deps {
        let root_id = format!("@{}", dep.path_name);
        loader
            .packages
            .insert(dep.path_name.clone(), root_id.clone());
        match &dep.source {
            Some(src) => {
                let root_file =
                    dep_root_file(src).map_err(|e| e.with_file(package_dir.join("xulo.toml")))?;
                let canonical = root_file.canonicalize().map_err(|e| {
                    XuloError::new(
                        ErrorKind::Io,
                        format!("cannot resolve {}: {e}", root_file.display()),
                    )
                })?;
                if let Some(prev) = loader.claimed.get(&canonical) {
                    return Err(dup_claim(&canonical, prev, &canonical, None));
                }
                loader.claimed.insert(canonical.clone(), root_id.clone());
                let home = canonical
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("."));
                let (program, source) = parse_file(&canonical)?;
                loader.walk(root_id, program, source, canonical, home)?;
            }
            None => loader.external_roots.push(root_id),
        }
    }
    // The package's own tree grows from the entry file (home = its
    // directory; the entry file itself is claimed by the root module).
    loader.claimed.insert(entry.clone(), String::new());
    let home = entry
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let (program, source) = parse_file(&entry)?;
    loader.walk(String::new(), program, source, entry, home)?;
    let entry_idx = *loader
        .id_to_idx
        .get("")
        .expect("the entry module is pushed by its walk");

    // Faces for macro import resolution, built from the written AST; the
    // final world is rebuilt after expansion so generated declarations are
    // part of every module's interface.
    let raw_world = build_world(&loader)?;
    expand_macros(&mut loader, &raw_world)?;
    let world = build_world(&loader)?;
    Ok(LoadedModules {
        modules: loader.modules,
        entry: entry_idx,
        world,
    })
}

/// Analyze every module in initialization order against the package world,
/// refreshing each face with the pass's exports so later modules see
/// completed signatures. `check_entry` enforces the entry-point rules
/// (`E0608`/`E0609`) on the root module — the run command, not the checker
/// on libraries. Returns each module's warnings tagged with its file.
pub fn analyze(loaded: &mut LoadedModules, check_entry: bool) -> Result<Vec<XuloError>, XuloError> {
    loaded.world.check_entry = check_entry;
    let count = loaded.modules.len();
    let mut warnings = Vec::new();
    for idx in 0..count {
        loaded.world.current = loaded.modules[idx].id.clone();
        let result = analyze_module_with(&loaded.modules[idx].program, &loaded.world)
            .map_err(|e| e.with_file(loaded.modules[idx].file.clone()))?;
        let file = loaded.modules[idx].file.clone();
        for w in &result.warnings {
            warnings.push(w.clone().with_file(file.clone()));
        }
        // Refresh the face: phase-2 exports carry inferred signatures and
        // re-export completions the syntactic face could not know.
        if let Some(face) = loaded.world.modules.get_mut(&loaded.modules[idx].id) {
            face.exports = result.exported_symbols.clone();
            face.types = result.exported_types.clone();
            face.impls = result.impls.clone();
        }
        loaded.modules[idx].uses = result.use_bindings.clone();
        loaded.modules[idx].analysis = Some(result);
    }
    Ok(warnings)
}

/// Populate `Call.trait_impl` on every module's AST from its analysis'
/// trait-dispatch annotations, and `BinaryOp.list_concat` from the list-concat
/// annotations. The native interpreter reads these fields to call the mangled
/// `impl_{Trait}_{Type}_{method}` function / concatenate arrays. Run after
/// [`analyze`].
pub fn apply_trait_dispatch(loaded: &mut LoadedModules) {
    for module in &mut loaded.modules {
        if let Some(analysis) = &module.analysis {
            let dispatch = analysis.trait_dispatch.clone();
            xulo_semantic::apply_trait_dispatch(&mut module.program, &dispatch);
            let concat = analysis.list_concat.clone();
            xulo_semantic::apply_list_concat(&mut module.program, &concat);
        }
    }
}

/// Rewrite bare callees that resolved to a std built-in (`floor(…)` →
/// `std::math::floor(…)`) across every module. The interpreter resolves the
/// bare names from its `use` bindings, so only the IR backend needs this;
/// call it before [`crate::compile_modules_to_ir`].
pub fn apply_std_calls(loaded: &mut LoadedModules) {
    for module in &mut loaded.modules {
        if let Some(analysis) = &module.analysis {
            let calls = analysis.std_calls.clone();
            xulo_semantic::apply_std_calls(&mut module.program, &calls);
        }
    }
}

struct Loader {
    /// canonical file path -> module id: every claimed body, deps included.
    claimed: HashMap<PathBuf, String>,
    /// Post-order module list: dependencies first, children before parents.
    modules: Vec<Module>,
    /// module id -> index into `modules`.
    id_to_idx: HashMap<String, usize>,
    /// manifest path name -> package root module id (`@path_name`).
    packages: HashMap<String, String>,
    /// Version-only dependencies: a package root id with no sources (its
    /// face is external).
    external_roots: Vec<String>,
}

impl Loader {
    /// Register `program` as module `id` (home `home`, source file `file`),
    /// recursively claiming each bodyless `mod` declaration's file, then
    /// push it — so children precede parents in `modules`.
    fn walk(
        &mut self,
        id: String,
        program: Program,
        source: String,
        file: PathBuf,
        home: PathBuf,
    ) -> Result<(), XuloError> {
        let mut mods: Vec<(String, usize)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for statement in &program.statements {
            let Statement::Mod(m) = statement else {
                continue;
            };
            if !seen.insert(m.name.clone()) {
                return Err(XuloError::new(
                    ErrorKind::Semantic,
                    format!("`mod {}` is declared twice in this module", m.name),
                )
                .at(m.span.clone())
                .with_code("E0104")
                .with_file(file.clone()));
            }
            let child_id = child_module_id(&id, &m.name);
            if let Some(body) = &m.body {
                // Inline: same source file (its spans live there), home H/m.
                let child_home = home.join(&m.name);
                let child_program = Program {
                    statements: body.clone(),
                };
                self.walk(
                    child_id.clone(),
                    child_program,
                    source.clone(),
                    file.clone(),
                    child_home,
                )?;
            } else {
                let candidate = home.join(format!("{}.xulo", m.name));
                let canonical = candidate.canonicalize().map_err(|_| {
                    XuloError::new(
                        ErrorKind::Semantic,
                        format!(
                            "cannot resolve `mod {}`: no file at {}",
                            m.name,
                            candidate.display()
                        ),
                    )
                    .at(m.span.clone())
                    .with_code("E0607")
                    .with_file(file.clone())
                })?;
                if canonical == file {
                    return Err(XuloError::new(
                        ErrorKind::Semantic,
                        format!(
                            "cannot resolve `mod {}`: it would claim the file that declares it",
                            m.name
                        ),
                    )
                    .at(m.span.clone())
                    .with_code("E0610")
                    .with_file(file.clone()));
                }
                if let Some(prev) = self.claimed.get(&canonical) {
                    return Err(dup_claim(&canonical, prev, &file, Some(m.span.clone())));
                }
                self.claimed.insert(canonical.clone(), child_id.clone());
                // A file-backed module at D/S.xulo has home D/S/ — the
                // directory named after the file's stem (spec `modules/
                // source-files.md`).
                let stem = canonical
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&m.name)
                    .to_string();
                let child_home = canonical
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(stem);
                let (child_program, child_source) = parse_file(&canonical)?;
                self.walk(
                    child_id.clone(),
                    child_program,
                    child_source,
                    canonical,
                    child_home,
                )?;
            }
            let idx = *self
                .id_to_idx
                .get(&child_id)
                .expect("walk pushes its module after its children");
            mods.push((m.name.clone(), idx));
        }
        let has_main = program_has_main(&program);
        let idx = self.modules.len();
        self.id_to_idx.insert(id.clone(), idx);
        self.modules.push(Module {
            file,
            source,
            id,
            program,
            analysis: None,
            uses: Vec::new(),
            mods,
            has_main,
        });
        Ok(())
    }
}

fn dup_claim(
    candidate: &Path,
    previous_id: &str,
    file: &Path,
    span: Option<std::ops::Range<usize>>,
) -> XuloError {
    let prev = module_label(previous_id);
    let mut err = XuloError::new(
        ErrorKind::Semantic,
        format!(
            "cannot claim {}: the file is already the body of {prev}",
            candidate.display()
        ),
    )
    .with_code("E0610")
    .with_file(file.to_path_buf());
    if let Some(span) = span {
        err = err.at(span);
    }
    err
}

/// A module id as diagnostics name it (`""` reads as the package root).
fn module_label(id: &str) -> String {
    if id.is_empty() {
        "the package root module".to_string()
    } else {
        format!("module `{id}`")
    }
}

fn parse_file(file: &Path) -> Result<(Program, String), XuloError> {
    let source = std::fs::read_to_string(file).map_err(|e| {
        XuloError::new(
            ErrorKind::Io,
            format!("cannot read {}: {e}", file.display()),
        )
    })?;
    let program =
        xulo_parser::parse_program(&source).map_err(|e| e.with_file(file.to_path_buf()))?;
    Ok((program, source))
}

/// The macro namespace of one loaded module, from the written AST.
struct ModuleMacros {
    /// Every macro the module declares, keyed by its macro name.
    own: HashMap<String, MacroDef>,
    /// The `pub` subset: what another module may import or reach by path.
    exports: HashMap<String, MacroDef>,
    /// A raw snapshot, so an imported implementation registers its defining
    /// module's functions (spec `builtins/execution.md`).
    raw: Program,
}

/// Expand every module's macros, importing macro names through `use`
/// (spec `modules/import-export.md`). Runs after preliminary faces exist and
/// before the final world is built, so generated declarations join the
/// interface.
fn expand_macros(loader: &mut Loader, world: &ModuleWorld) -> Result<(), XuloError> {
    let mut macros: HashMap<String, ModuleMacros> = HashMap::new();
    for module in &loader.modules {
        let registry =
            collect_macros(&module.program).map_err(|e| e.with_file(module.file.clone()))?;
        let exported_fns = pub_macro_fn_names(&module.program);
        let own = registry.defs;
        let exports = own
            .iter()
            .filter(|(_, def)| exported_fns.contains(&def.fn_name))
            .map(|(name, def)| (name.clone(), def.clone()))
            .collect();
        macros.insert(
            module.id.clone(),
            ModuleMacros {
                own,
                exports,
                raw: module.program.clone(),
            },
        );
    }

    for idx in 0..loader.modules.len() {
        let id = loader.modules[idx].id.clone();
        let source = loader.modules[idx].source.clone();
        let file = loader.modules[idx].file.clone();
        let mut seen: HashSet<String> = macros
            .get(&id)
            .map(|m| m.own.keys().cloned().collect())
            .unwrap_or_default();
        let mut imports: Vec<(String, MacroDef)> = Vec::new();
        let mut reexports: Vec<(String, MacroDef)> = Vec::new();
        let mut namespaces: Vec<(String, String)> = Vec::new();

        for statement in &loader.modules[idx].program.statements {
            let Statement::Use(u) = statement else {
                continue;
            };
            if u.type_only {
                continue;
            }
            let resolved = resolve_use_macros(&id, u, world, &macros)?;
            for (local, target, member) in resolved.members {
                let Some(source_info) = macros.get(&target) else {
                    // A version-only manifest dependency has no sources: its
                    // members bind opaquely (no macro can expand).
                    continue;
                };
                let def = if target == id {
                    source_info.own.get(&member)
                } else {
                    source_info.exports.get(&member)
                };
                let Some(def) = def else {
                    if target != id && source_info.own.contains_key(&member) {
                        let owner = if target.is_empty() {
                            "the package root module".to_string()
                        } else {
                            format!("module `{target}`")
                        };
                        return Err(XuloError::new(
                            ErrorKind::Semantic,
                            format!("macro `{member}` is private to {owner}"),
                        )
                        .with_code("E0602")
                        .at(u.span.clone())
                        .with_file(file.clone()));
                    }
                    continue;
                };
                let mut def = def.clone();
                if target != id && def.exec_base.is_none() {
                    def.exec_base = Some(Rc::new(source_info.raw.clone()));
                }
                if !seen.insert(local.clone()) {
                    return Err(XuloError::new(
                        ErrorKind::Semantic,
                        format!("`{local}` is bound twice in this module's macro namespace"),
                    )
                    .with_code("E0104")
                    .at(u.span.clone())
                    .with_file(file.clone()));
                }
                if u.is_pub {
                    reexports.push((local.clone(), def.clone()));
                }
                imports.push((local, def));
            }
            for target in resolved.globs {
                let Some(source_info) = macros.get(&target) else {
                    continue;
                };
                for (name, def) in &source_info.exports {
                    let mut def = def.clone();
                    if def.exec_base.is_none() {
                        def.exec_base = Some(Rc::new(source_info.raw.clone()));
                    }
                    if !seen.insert(name.clone()) {
                        continue;
                    }
                    if u.is_pub {
                        reexports.push((name.clone(), def.clone()));
                    }
                    imports.push((name.clone(), def));
                }
            }
            namespaces.extend(resolved.namespaces);
            if u.is_pub
                && let UseTarget::Local(names) = &u.target
            {
                for name in names {
                    let def = macros
                        .get(&id)
                        .and_then(|info| info.own.get(name))
                        .or_else(|| {
                            imports
                                .iter()
                                .rev()
                                .find(|(local, _)| local == name)
                                .map(|(_, def)| def)
                        })
                        .cloned();
                    if let Some(def) = def {
                        reexports.push((name.clone(), def));
                    }
                }
            }
        }
        if let Some(info) = macros.get_mut(&id) {
            for (name, def) in reexports {
                info.exports.insert(name, def);
            }
        }

        // Qualified function-macro paths (`m::query!`) resolve against every
        // visible module path; a private macro is simply absent.
        let mut qualified: Vec<(String, MacroDef)> = Vec::new();
        let mut private_qualified: HashSet<String> = HashSet::new();
        let mut push_qualified = |path: String, def: &MacroDef, raw: &Program| {
            let mut def = def.clone();
            if def.exec_base.is_none() {
                def.exec_base = Some(Rc::new(raw.clone()));
            }
            qualified.push((path, def));
        };
        for (module_id, info) in &macros {
            if module_id.is_empty() {
                continue;
            }
            for (name, def) in &info.exports {
                push_qualified(format!("{module_id}::{name}"), def, &info.raw);
                if let Some(pkg) = module_id.strip_prefix('@')
                    && let Some((pkg, _)) = pkg.split_once("::")
                {
                    push_qualified(format!("{pkg}::{name}"), def, &info.raw);
                }
            }
            for name in info.own.keys() {
                if info.exports.contains_key(name) {
                    continue;
                }
                private_qualified.insert(format!("{module_id}::{name}"));
                if let Some(pkg) = module_id.strip_prefix('@')
                    && let Some((pkg, _)) = pkg.split_once("::")
                {
                    private_qualified.insert(format!("{pkg}::{name}"));
                }
            }
        }
        for (alias, target) in namespaces {
            if let Some(info) = macros.get(&target) {
                for (name, def) in &info.exports {
                    push_qualified(format!("{alias}::{name}"), def, &info.raw);
                }
                for name in info.own.keys() {
                    if !info.exports.contains_key(name) {
                        private_qualified.insert(format!("{alias}::{name}"));
                    }
                }
            }
        }

        expand_program_with_imports(
            &mut loader.modules[idx].program,
            &source,
            &file.display().to_string(),
            &imports,
            &qualified,
            &private_qualified,
        )
        .map_err(|e| e.with_file(file))?;
    }
    Ok(())
}

/// The identifiers of `pub fn`s carrying `#[macro]` — the macro names a
/// module exports (spec `modules/visibility.md`).
fn pub_macro_fn_names(program: &Program) -> HashSet<String> {
    let mut out = HashSet::new();
    for statement in &program.statements {
        if let Statement::Export(export) = statement
            && let ExportItem::Fn(f) = &export.item
            && f.attributes.iter().any(|a| a.name == "macro")
        {
            out.insert(f.name.clone());
        }
    }
    out
}

struct ResolvedUse {
    /// `(local name, target module, member name)` for a named macro import.
    members: Vec<(String, String, String)>,
    /// Target modules whose whole `pub` macro set a glob imports.
    globs: Vec<String>,
    /// `use m;` / `use m as ns;` or a path ending at a child module.
    namespaces: Vec<(String, String)>,
}

fn resolve_use_macros(
    current: &str,
    u: &UseDecl,
    world: &ModuleWorld,
    _macros: &HashMap<String, ModuleMacros>,
) -> Result<ResolvedUse, XuloError> {
    let mut out = ResolvedUse {
        members: Vec::new(),
        globs: Vec::new(),
        namespaces: Vec::new(),
    };
    match &u.target {
        UseTarget::Path {
            head,
            segments,
            alias,
        } => {
            let Some(start) = resolve_use_head(current, head, world) else {
                return Ok(out);
            };
            if segments.is_empty() {
                let local = alias.clone().unwrap_or_else(|| head_default(head));
                out.namespaces.push((local, start));
                return Ok(out);
            }
            if let Some((target, member)) = walk_module_path(&start, segments, world) {
                match member {
                    Some(member) => {
                        let local = alias.clone().unwrap_or_else(|| member.clone());
                        out.members.push((local, target, member));
                    }
                    None => {
                        let local = alias.clone().unwrap_or_else(|| {
                            segments
                                .last()
                                .cloned()
                                .unwrap_or_else(|| head_default(head))
                        });
                        out.namespaces.push((local, target));
                    }
                }
            }
            Ok(out)
        }
        UseTarget::Group { head, entries } => {
            let Some(start) = resolve_use_head(current, head, world) else {
                return Ok(out);
            };
            for entry in entries {
                if let Some((target, Some(member))) =
                    walk_module_path(&start, &entry.segments, world)
                {
                    let local = entry.alias.clone().unwrap_or_else(|| member.clone());
                    out.members.push((local, target, member));
                } else if let Some((target, None)) =
                    walk_module_path(&start, &entry.segments, world)
                {
                    let local = entry
                        .alias
                        .clone()
                        .unwrap_or_else(|| entry.segments.last().cloned().unwrap_or_default());
                    out.namespaces.push((local, target));
                }
            }
            Ok(out)
        }
        UseTarget::Glob { head, segments } => {
            let Some(start) = resolve_use_head(current, head, world) else {
                return Ok(out);
            };
            if segments.is_empty() {
                out.globs.push(start);
            } else if let Some((target, None)) = walk_module_path(&start, segments, world) {
                out.globs.push(target);
            }
            Ok(out)
        }
        // `pub use { name }`: the name is already bound here; a macro binding
        // is already in `exports` from its own declaration or earlier import.
        UseTarget::Local(_) => Ok(out),
    }
}

fn head_default(head: &PathHead) -> String {
    match head {
        PathHead::Name(n) | PathHead::Package(n) => n.clone(),
        PathHead::SelfModule => "self".to_string(),
        PathHead::Super => "super".to_string(),
    }
}

fn resolve_use_head(current: &str, head: &PathHead, world: &ModuleWorld) -> Option<String> {
    match head {
        PathHead::SelfModule => Some(current.to_string()),
        PathHead::Super => current
            .rsplit_once("::")
            .map(|(parent, _)| parent.to_string())
            .or_else(|| Some(String::new())),
        PathHead::Name(n) | PathHead::Package(n) => {
            let root = ModuleWorld::package_root_of(current);
            if root.is_empty()
                && let Some(id) = world.packages.get(n)
            {
                return Some(id.clone());
            }
            world
                .modules
                .get(root)
                .and_then(|face| face.children.get(n))
                .cloned()
        }
    }
}

/// Walk `start` through `segments`: returns the target module and, when the
/// last segment names a member rather than a child module, that member.
fn walk_module_path(
    start: &str,
    segments: &[String],
    world: &ModuleWorld,
) -> Option<(String, Option<String>)> {
    let mut current = start.to_string();
    for (i, seg) in segments.iter().enumerate() {
        let face = world.modules.get(&current)?;
        let child = face.children.get(seg).cloned();
        if i + 1 == segments.len() {
            return match child {
                Some(child) => Some((child, None)),
                None => Some((current, Some(seg.clone()))),
            };
        }
        current = child?;
    }
    Some((current, None))
}

/// Does this module declare a `main` function (directly or via `pub`)?
fn program_has_main(program: &Program) -> bool {
    fn fn_named(f: &xulo_core::ast::FnDef) -> bool {
        f.name == "main"
    }
    program.statements.iter().any(|s| match s {
        Statement::Fn(f) => fn_named(f),
        Statement::Export(export) => match &export.item {
            ExportItem::Fn(f) => fn_named(f),
            _ => false,
        },
        _ => false,
    })
}

/// The source file a path dependency designates: `<dir>/main.xulo` for a
/// directory, the file itself when a file is named.
fn dep_root_file(src: &Path) -> Result<PathBuf, XuloError> {
    if src.is_file() {
        return Ok(src.to_path_buf());
    }
    let root = src.join("main.xulo");
    if root.is_file() {
        return Ok(root);
    }
    Err(XuloError::new(
        ErrorKind::Semantic,
        format!(
            "cannot resolve dependency {}: no main.xulo under {}",
            src.display(),
            src.display()
        ),
    ))
}

/// One `[dependencies]` entry: the source-visible path name and, for a
/// locally pathed dependency, where its sources live.
struct ManifestDep {
    path_name: String,
    source: Option<PathBuf>,
}

/// The package's dependency mapping (spec `modules/import-export.md`,
/// "Package names and the manifest"): `[dependencies]` in `xulo.toml` beside
/// the entry file, each key mapped to its path name (`alias` replaces the
/// key) with an optional `path` for local sources.
fn parse_manifest(package_dir: &Path) -> Result<Vec<ManifestDep>, XuloError> {
    let manifest = package_dir.join("xulo.toml");
    if !manifest.is_file() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&manifest).map_err(|e| {
        XuloError::new(
            ErrorKind::Io,
            format!("cannot read {}: {e}", manifest.display()),
        )
        .with_file(manifest.clone())
    })?;
    let mut deps: Vec<ManifestDep> = Vec::new();
    let mut in_deps = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_deps = line == "[dependencies]";
            continue;
        }
        if !in_deps {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(manifest_err(
                &manifest,
                format!("cannot parse dependency line `{line}`"),
            ));
        };
        let key = key.trim().trim_matches('"').trim();
        let value = value.trim();
        let mut alias: Option<String> = None;
        let mut source: Option<PathBuf> = None;
        if value.starts_with('{') {
            let body = value
                .trim_start_matches('{')
                .trim_end_matches('}')
                .to_string();
            for field in split_top_level(&body) {
                let Some((fname, fval)) = field.split_once('=') else {
                    return Err(manifest_err(
                        &manifest,
                        format!("cannot parse dependency field `{field}`"),
                    ));
                };
                let fname = fname.trim();
                let fval = fval.trim().trim_matches('"');
                match fname {
                    "alias" => alias = Some(fval.to_string()),
                    "path" => source = Some(package_dir.join(fval)),
                    "version" => {}
                    other => {
                        return Err(manifest_err(
                            &manifest,
                            format!("unknown field `{other}` in dependency `{key}`"),
                        ));
                    }
                }
            }
        } else if !value.starts_with('"') {
            return Err(manifest_err(
                &manifest,
                format!("dependency `{key}` must be a version string or an inline table"),
            ));
        }
        let path_name = alias.unwrap_or_else(|| key.to_string());
        if !is_path_name(&path_name) {
            return Err(manifest_err(
                &manifest,
                format!("dependency `{key}` needs an `alias` in the manifest"),
            ));
        }
        if deps.iter().any(|d| d.path_name == path_name) {
            return Err(manifest_err(
                &manifest,
                format!("two dependencies share the path name `{path_name}`"),
            ));
        }
        deps.push(ManifestDep { path_name, source });
    }
    Ok(deps)
}

fn manifest_err(manifest: &Path, message: String) -> XuloError {
    XuloError::new(ErrorKind::Semantic, message).with_file(manifest.to_path_buf())
}

/// Split an inline-table body on its top-level commas (quotes and nested
/// `{ … }` held out).
fn split_top_level(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut start = 0;
    for (i, ch) in body.char_indices() {
        match ch {
            '"' => in_str = !in_str,
            '{' if !in_str => depth += 1,
            '}' if !in_str => depth = depth.saturating_sub(1),
            ',' if !in_str && depth == 0 => {
                out.push(body[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let tail = body[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Is this spelling usable as a path head: an identifier or kebab-case
/// (`lsp-server`); anything else (`@xulo/ui`) needs an `alias`.
fn is_path_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.split('-').all(|part| {
            let mut chars = part.chars();
            matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
                && chars.all(|c| c.is_alphanumeric() || c == '_')
        })
}

/// Extract every module's face, complete `pub use` re-exports to a
/// fixpoint (so `use` cycles stay ordinary), and assemble the world.
fn build_world(loader: &Loader) -> Result<ModuleWorld, XuloError> {
    let faces: Vec<ModuleFace> = loader
        .modules
        .iter()
        .map(|m| module_face(&m.program, &m.id))
        .collect();
    let mut world = ModuleWorld {
        root: String::new(),
        packages: loader.packages.clone(),
        ..ModuleWorld::default()
    };
    for (i, m) in loader.modules.iter().enumerate() {
        world.init_order.insert(m.id.clone(), i);
        world.modules.insert(m.id.clone(), faces[i].clone());
    }
    for id in &loader.external_roots {
        world.modules.insert(
            id.clone(),
            ModuleFace {
                external: true,
                ..ModuleFace::default()
            },
        );
    }
    // Local namespaces begin as this module's `mod` declarations; the
    // fixpoint adds what `use` binds, so a `pub use` head resolves no
    // matter which header form binds it.
    let mut local_ns: Vec<HashMap<String, String>> =
        faces.iter().map(|f| f.children.clone()).collect();
    complete_faces(loader, &mut world, &mut local_ns);
    Ok(world)
}

/// A pending `pub use` completion for one local name.
enum Reexport {
    /// `pub use head;` (or a path ending at a child module): the namespace.
    Namespace { local: String, target: String },
    /// `pub use head::…::member` (or a glob entry): copy the member.
    Member {
        local: String,
        target: String,
        member: String,
    },
    /// A path into an external dependency: an opaque value binding.
    Opaque { local: String },
}

/// Complete faces for `pub use` to a fixpoint: register every header-form
/// namespace binding (private ones too — later heads resolve through them),
/// and for each `pub` re-export copy the target's kind/symbol/type (or
/// namespace) into this module's face. Iterates until nothing changes, so
/// `use` cycles need no ordering (spec `modules/import-export.md`).
fn complete_faces(
    loader: &Loader,
    world: &mut ModuleWorld,
    local_ns: &mut [HashMap<String, String>],
) {
    // Chains settle in a few rounds; the cap only guards a pathological
    // rewrite cycle, which phase-2 would reject as E0101/E0601 anyway.
    for _ in 0..64 {
        let mut changed = false;
        for i in 0..loader.modules.len() {
            let module_id = loader.modules[i].id.clone();
            let mut ns_updates: Vec<(String, String)> = Vec::new();
            let mut upgrades: Vec<Reexport> = Vec::new();
            // Local name -> the module member it was imported from, for
            // `pub use { name }` of a macro binding (macros have no value
            // symbol to copy, so the member origin is needed).
            let mut macro_origin: HashMap<String, (String, String)> = HashMap::new();
            for statement in &loader.modules[i].program.statements {
                let Statement::Use(u) = statement else {
                    continue;
                };
                if u.type_only {
                    continue;
                }
                let entries: Vec<(&PathHead, &[String], Option<&str>)> = match &u.target {
                    UseTarget::Path {
                        head,
                        segments,
                        alias,
                    } => vec![(head, segments.as_slice(), alias.as_deref())],
                    UseTarget::Group { head, entries } => entries
                        .iter()
                        .map(|e| (head, e.segments.as_slice(), e.alias.as_deref()))
                        .collect(),
                    _ => Vec::new(),
                };
                for (head, segments, alias) in entries {
                    if segments.is_empty() {
                        continue;
                    }
                    let Some(start) = fixup_head(head, i, &module_id, world, local_ns) else {
                        continue;
                    };
                    let FaceWalk::Found { target, member } = walk_face(world, &start, segments)
                    else {
                        continue;
                    };
                    let is_macro = world
                        .modules
                        .get(&target)
                        .is_some_and(|f| f.macros.contains_key(&member));
                    if !is_macro {
                        continue;
                    }
                    let local = alias.map(str::to_string).unwrap_or_else(|| member.clone());
                    macro_origin.insert(local, (target, member));
                }
            }
            for statement in &loader.modules[i].program.statements {
                let Statement::Use(u) = statement else {
                    continue;
                };
                if u.type_only {
                    continue;
                }
                match &u.target {
                    UseTarget::Path {
                        head,
                        segments,
                        alias,
                    } => {
                        plan_path(
                            head,
                            segments,
                            alias.as_deref(),
                            u,
                            &module_id,
                            i,
                            world,
                            local_ns,
                            &mut ns_updates,
                            &mut upgrades,
                        );
                    }
                    UseTarget::Group { head, entries } => {
                        for entry in entries {
                            plan_path(
                                head,
                                &entry.segments,
                                entry.alias.as_deref(),
                                u,
                                &module_id,
                                i,
                                world,
                                local_ns,
                                &mut ns_updates,
                                &mut upgrades,
                            );
                        }
                    }
                    UseTarget::Glob { head, .. } => {
                        let Some(start) = fixup_head(head, i, &module_id, world, local_ns) else {
                            continue;
                        };
                        let Some(target_face) = world.modules.get(&start).cloned() else {
                            continue;
                        };
                        if target_face.external {
                            continue;
                        }
                        let members: Vec<(String, FaceName)> = target_face
                            .names
                            .iter()
                            .filter(|(_, info)| info.is_pub)
                            .map(|(n, info)| (n.clone(), *info))
                            .collect();
                        for (name, info) in members {
                            if info.kind == FaceKind::Module
                                && let Some(child) = target_face.children.get(&name)
                            {
                                ns_updates.push((name.clone(), child.clone()));
                            }
                            if u.is_pub {
                                upgrades.push(Reexport::Member {
                                    local: name.clone(),
                                    target: start.clone(),
                                    member: name,
                                });
                            }
                        }
                        if u.is_pub {
                            let macros: Vec<String> = target_face
                                .macros
                                .iter()
                                .filter(|(_, is_pub)| **is_pub)
                                .map(|(n, _)| n.clone())
                                .collect();
                            for name in macros {
                                upgrades.push(Reexport::Member {
                                    local: name.clone(),
                                    target: start.clone(),
                                    member: name,
                                });
                            }
                        }
                    }
                    UseTarget::Local(names) => {
                        // The local form names bindings of this module;
                        // `module_face` already marked them pub. A macro
                        // binding has no value symbol, so complete it from
                        // the member it was imported from.
                        if u.is_pub {
                            for name in names {
                                if let Some((target, member)) = macro_origin.get(name) {
                                    upgrades.push(Reexport::Member {
                                        local: name.clone(),
                                        target: target.clone(),
                                        member: member.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
            for (local, id) in ns_updates {
                if insert_changed(&mut local_ns[i], local, id) {
                    changed = true;
                }
            }
            if !upgrades.is_empty() {
                let mut face = world.modules.get(&module_id).cloned().unwrap_or_default();
                for upgrade in upgrades {
                    if apply_reexport(&mut face, upgrade, world) {
                        changed = true;
                    }
                }
                world.modules.insert(module_id.clone(), face);
            }
        }
        if !changed {
            break;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn plan_path(
    head: &PathHead,
    segments: &[String],
    alias: Option<&str>,
    u: &UseDecl,
    module_id: &str,
    idx: usize,
    world: &ModuleWorld,
    local_ns: &[HashMap<String, String>],
    ns_updates: &mut Vec<(String, String)>,
    upgrades: &mut Vec<Reexport>,
) {
    let Some(start) = fixup_head(head, idx, module_id, world, local_ns) else {
        return;
    };
    if segments.is_empty() {
        // `use form;` — bind the namespace itself.
        let local = alias
            .map(str::to_string)
            .unwrap_or_else(|| head_local(head));
        ns_updates.push((local.clone(), start.clone()));
        if u.is_pub {
            upgrades.push(Reexport::Namespace {
                local,
                target: start,
            });
        }
        return;
    }
    let local = alias
        .map(str::to_string)
        .unwrap_or_else(|| segments.last().cloned().expect("segments non-empty"));
    match walk_face(world, &start, segments) {
        FaceWalk::External => {
            if u.is_pub {
                upgrades.push(Reexport::Opaque { local });
            }
        }
        FaceWalk::Fail => {}
        FaceWalk::Found { target, member } => {
            let target_face = world.modules.get(&target);
            if let Some(child) = target_face.and_then(|f| f.children.get(&member)) {
                // The path names a child module: the local is a namespace.
                ns_updates.push((local.clone(), child.clone()));
            }
            if u.is_pub {
                upgrades.push(Reexport::Member {
                    local,
                    target,
                    member,
                });
            }
        }
    }
}

enum FaceWalk {
    Found { target: String, member: String },
    External,
    Fail,
}

/// Walk `segments` from `start` through module children: the last segment
/// is the member, intermediate ones must be children. Any external face
/// stops the walk — its content is invisible.
fn walk_face(world: &ModuleWorld, start: &str, segments: &[String]) -> FaceWalk {
    let mut current = start.to_string();
    for (k, seg) in segments.iter().enumerate() {
        let Some(face) = world.modules.get(&current) else {
            return FaceWalk::Fail;
        };
        if face.external {
            return FaceWalk::External;
        }
        if k + 1 == segments.len() {
            return FaceWalk::Found {
                target: current,
                member: seg.clone(),
            };
        }
        match face.children.get(seg) {
            Some(child) => current = child.clone(),
            None => return FaceWalk::Fail,
        }
    }
    FaceWalk::Fail
}

/// Resolve a `use` head: `self`/`super`, a local namespace binding, then
/// (this package only) a manifest package, then a root module of *this*
/// package — bare paths are package-root absolute.
fn fixup_head(
    head: &PathHead,
    idx: usize,
    module_id: &str,
    world: &ModuleWorld,
    local_ns: &[HashMap<String, String>],
) -> Option<String> {
    match head {
        PathHead::SelfModule => Some(module_id.to_string()),
        PathHead::Super => {
            let parent = module_id
                .rsplit_once("::")
                .map(|(p, _)| p.to_string())
                .unwrap_or_default();
            // Above the package root, `super` resolves to nothing
            // (phase-2 reports E0607).
            if parent.is_empty()
                && (module_id.is_empty() || ModuleWorld::package_root_of(module_id) == module_id)
            {
                None
            } else {
                Some(parent)
            }
        }
        PathHead::Name(n) | PathHead::Package(n) => {
            if let Some(id) = local_ns[idx].get(n) {
                return Some(id.clone());
            }
            let pkg_root = ModuleWorld::package_root_of(module_id);
            if pkg_root.is_empty()
                && let Some(id) = world.packages.get(n)
            {
                return Some(id.clone());
            }
            world
                .modules
                .get(pkg_root)
                .and_then(|f| f.children.get(n))
                .cloned()
        }
    }
}

/// Merge one completed re-export into a face; returns whether it changed
/// anything (the fixpoint's termination signal).
fn apply_reexport(face: &mut ModuleFace, upgrade: Reexport, world: &ModuleWorld) -> bool {
    match upgrade {
        Reexport::Namespace { local, target } => {
            let mut changed = name_upsert(
                face,
                &local,
                FaceName {
                    is_pub: true,
                    kind: FaceKind::Module,
                },
            );
            if insert_changed(&mut face.children, local, target) {
                changed = true;
            }
            changed
        }
        Reexport::Opaque { local } => name_upsert(
            face,
            &local,
            FaceName {
                is_pub: true,
                kind: FaceKind::Value,
            },
        ),
        Reexport::Member {
            local,
            target,
            member,
        } => {
            let Some(target_face) = world.modules.get(&target) else {
                return false;
            };
            let mut changed = false;
            if let Some(&is_pub) = target_face.macros.get(&member)
                && is_pub
            {
                match face.macros.get(&local) {
                    Some(existing) if *existing => {}
                    _ => {
                        face.macros.insert(local.clone(), true);
                        changed = true;
                    }
                }
            }
            let Some(info) = target_face.names.get(&member).copied() else {
                return changed;
            };
            changed |= name_upsert(
                face,
                &local,
                FaceName {
                    is_pub: true,
                    kind: info.kind,
                },
            );
            match info.kind {
                FaceKind::Module => {
                    if let Some(child) = target_face.children.get(&member)
                        && insert_changed(&mut face.children, local, child.clone())
                    {
                        changed = true;
                    }
                }
                FaceKind::Type => {
                    if let Some((_, entry)) = target_face.types.iter().find(|(n, _)| *n == member)
                        && upsert_pair(&mut face.types, local, entry.clone())
                    {
                        changed = true;
                    }
                }
                FaceKind::Fn | FaceKind::Let | FaceKind::Value => {
                    if let Some((_, symbol)) =
                        target_face.exports.iter().find(|(n, _)| *n == member)
                        && upsert_pair(&mut face.exports, local, symbol.clone())
                    {
                        changed = true;
                    }
                }
            }
            changed
        }
    }
}

fn name_upsert(face: &mut ModuleFace, local: &str, value: FaceName) -> bool {
    match face.names.get(local) {
        Some(existing) if *existing == value => false,
        _ => {
            face.names.insert(local.to_string(), value);
            true
        }
    }
}

fn insert_changed(map: &mut HashMap<String, String>, key: String, value: String) -> bool {
    match map.get(&key) {
        Some(existing) if *existing == value => false,
        _ => {
            map.insert(key, value);
            true
        }
    }
}

fn upsert_pair<T: PartialEq>(pairs: &mut Vec<(String, T)>, key: String, value: T) -> bool {
    if let Some(slot) = pairs.iter_mut().find(|(n, _)| *n == key) {
        if slot.1 == value {
            return false;
        }
        slot.1 = value;
        return true;
    }
    pairs.push((key, value));
    true
}

/// A path head's written name, as a default local binding.
fn head_local(head: &PathHead) -> String {
    match head {
        PathHead::Name(n) | PathHead::Package(n) => n.clone(),
        PathHead::SelfModule => "self".to_string(),
        PathHead::Super => "super".to_string(),
    }
}
