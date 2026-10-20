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

use xulo_core::ast::{ExportItem, PathHead, Program, Statement, UseDecl, UseTarget};
use xulo_core::error::{ErrorKind, XuloError};
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
                let program = parse_file(&canonical)?;
                loader.walk(root_id, program, canonical, home)?;
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
    let program = parse_file(&entry)?;
    loader.walk(String::new(), program, entry, home)?;
    let entry_idx = *loader
        .id_to_idx
        .get("")
        .expect("the entry module is pushed by its walk");

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
                self.walk(child_id.clone(), child_program, file.clone(), child_home)?;
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
                let child_program = parse_file(&canonical)?;
                self.walk(child_id.clone(), child_program, canonical, child_home)?;
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

fn parse_file(file: &Path) -> Result<Program, XuloError> {
    let source = std::fs::read_to_string(file).map_err(|e| {
        XuloError::new(
            ErrorKind::Io,
            format!("cannot read {}: {e}", file.display()),
        )
    })?;
    xulo_parser::parse_program(&source).map_err(|e| e.with_file(file.to_path_buf()))
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
                    }
                    UseTarget::Local(_) => {
                        // The local form names bindings of this module;
                        // `module_face` already marked them pub.
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
            let Some(info) = target_face.names.get(&member).copied() else {
                return false;
            };
            let mut changed = name_upsert(
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
