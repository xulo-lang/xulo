//! Multi-file module loading.
//!
//! `import` statements with a specifier that resolves to a local `.xulo` file
//! (relative paths, or a bare name that exists next to the importer) are loaded
//! and semantically analyzed in dependency order, so the native interpreter can
//! execute them module by module. Any other specifier is treated as an external
//! package (with no native value; the CLI binds its names to `null`
//! placeholders). There is no JavaScript backend anymore: programs run on the
//! native Rust interpreter.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use xulo_core::ast::{ExportItem, ImportSpec, ImportStmt, Program, Statement, Type};
use xulo_core::error::{ErrorKind, XuloError};
use xulo_semantic::symbol_table::{Symbol, SymbolKind};
use xulo_semantic::{AnalysisResult, TypeEntry, TypeEntryKind, analyze_with};

/// One loaded module, in dependency (topological) order.
pub struct Module {
    pub file: PathBuf,
    pub program: Program,
    pub analysis: Option<AnalysisResult>,
    /// Imports that resolve to another bundled module.
    pub imports: Vec<ImportBinding>,
    /// Whether this module declares a `main` function (the entry runs it).
    pub has_main: bool,
}

/// An import that resolves to another bundled module.
#[derive(Debug, Clone)]
pub struct ImportBinding {
    pub target: usize,
    pub spec: ImportSpec,
    pub type_only: bool,
}

/// The result of loading an entry file and its transitive local imports.
pub struct LoadedModules {
    /// Dependencies precede dependents.
    pub modules: Vec<Module>,
    /// Non-local imports emitted verbatim as ESM at the top of the bundle.
    pub external_imports: Vec<ImportStmt>,
    /// Index of the entry module: its `main()` runs when the bundle is loaded.
    pub entry: usize,
}

/// Load, analyze, and annotate the module graph reachable from `entry`.
/// Returns the non-fatal warnings raised during analysis (each as
/// `(file, message)`).
pub fn compile_file(entry: &Path) -> Result<Vec<XuloError>, XuloError> {
    let mut loaded = load(entry)?;
    let warnings = analyze(&mut loaded)?;
    apply_trait_dispatch(&mut loaded);
    Ok(warnings)
}

/// Resolve the transitive local imports of `entry`.
pub fn load(entry: &Path) -> Result<LoadedModules, XuloError> {
    let entry = entry.canonicalize().map_err(|e| {
        XuloError::new(
            ErrorKind::Io,
            format!("cannot resolve {}: {e}", entry.display()),
        )
    })?;
    let mut loader = Loader {
        index: HashMap::new(),
        loading: HashSet::new(),
        modules: Vec::new(),
        external_imports: Vec::new(),
    };
    let entry_index = loader.load_file(&entry)?;
    Ok(LoadedModules {
        modules: loader.modules,
        external_imports: loader.external_imports,
        entry: entry_index,
    })
}

struct Loader {
    /// canonical file path -> module index (only fully-loaded modules).
    index: HashMap<PathBuf, usize>,
    /// files on the current DFS stack (for cycle detection).
    loading: HashSet<PathBuf>,
    modules: Vec<Module>,
    external_imports: Vec<ImportStmt>,
}

impl Loader {
    fn load_file(&mut self, file: &Path) -> Result<usize, XuloError> {
        let file = file.canonicalize().map_err(|e| {
            XuloError::new(
                ErrorKind::Io,
                format!("cannot resolve {}: {e}", file.display()),
            )
        })?;
        if let Some(&idx) = self.index.get(&file) {
            return Ok(idx);
        }
        if self.loading.contains(&file) {
            return Err(XuloError::new(
                ErrorKind::Semantic,
                format!("circular import involving {}", file.display()),
            ));
        }
        let source = std::fs::read_to_string(&file).map_err(|e| {
            XuloError::new(
                ErrorKind::Io,
                format!("cannot read {}: {e}", file.display()),
            )
        })?;
        let tokens = xulo_lexer::tokenize(&source).map_err(|e| e.with_file(file.clone()))?;
        let program = xulo_parser::parse_program(&tokens).map_err(|e| e.with_file(file.clone()))?;

        self.loading.insert(file.clone());
        let base = file.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut imports = Vec::new();
        for statement in &program.statements {
            if let Statement::Import(imp) = statement {
                match resolve_local(&base, &imp.source) {
                    Some(target) => {
                        let idx = self.load_file(&target)?;
                        imports.push(ImportBinding {
                            target: idx,
                            spec: imp.spec.clone(),
                            type_only: imp.type_only,
                        });
                    }
                    None => self.external_imports.push(imp.clone()),
                }
            }
        }
        self.loading.remove(&file);

        let has_main = program_has_main(&program);
        let idx = self.modules.len();
        self.index.insert(file.clone(), idx);
        self.modules.push(Module {
            file,
            program,
            analysis: None,
            imports,
            has_main,
        });
        Ok(idx)
    }
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

/// Try to resolve an import specifier against a local `.xulo` file. Returns
/// `None` when the specifier names an external package.
fn resolve_local(base: &Path, source: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    let joined = base.join(source);
    candidates.push(joined.clone());
    candidates.push(PathBuf::from(format!("{}.xulo", joined.display())));
    candidates.push(joined.join("index.xulo"));
    candidates.into_iter().find(|p| p.is_file())
}

/// Analyze every module in dependency order, seeding each one with the
/// symbols and types its imports pull in. Rejects imports of names the target
/// does not export. Returns each module's warnings tagged with its file.
pub fn analyze(loaded: &mut LoadedModules) -> Result<Vec<XuloError>, XuloError> {
    let modules = &mut loaded.modules;
    let count = modules.len();
    let mut warnings = Vec::new();
    for idx in 0..count {
        let (symbols, types, impls) = collect_imports(modules, idx)?;
        let result = analyze_with(&modules[idx].program, &symbols, &types, &impls)
            .map_err(|e| e.with_file(modules[idx].file.clone()))?;
        for w in &result.warnings {
            warnings.push(w.clone().with_file(modules[idx].file.clone()));
        }
        modules[idx].analysis = Some(result);
    }
    Ok(warnings)
}

/// Populate `Call.trait_impl` on every module's AST from its analysis'
/// trait-dispatch annotations, and `BinaryOp.list_concat` from the list-concat
/// annotations. Codegen and the native interpreter read these fields to emit
/// the mangled `impl_{Trait}_{Type}_{method}` call / array concatenation. Run
/// after [`analyze`].
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

/// Imports' symbols and type entries gathered from a module's dependencies,
/// plus `impl` registrations (`(trait, type, method)`) for the imported
/// receiver types, so dispatch calls on them resolve in the importing module.
type ImportSeed = (
    Vec<Symbol>,
    Vec<(String, TypeEntry)>,
    Vec<(String, String, String)>,
);

fn collect_imports(modules: &[Module], idx: usize) -> Result<ImportSeed, XuloError> {
    let mut symbols: Vec<Symbol> = Vec::new();
    let mut types: Vec<(String, TypeEntry)> = Vec::new();
    let mut impls: Vec<(String, String, String)> = Vec::new();
    for binding in &modules[idx].imports {
        let target = &modules[binding.target];
        let analysis = target
            .analysis
            .as_ref()
            .expect("dependencies are analyzed before dependents");
        let target_readable = target.file.display();
        // Impls travel with the imported receiver type only when the name is
        // imported unaliased: dispatch mangles `impl_{Trait}_{Type}_{method}`
        // from the local names, which must match the declaring module's to
        // hit the registered function.
        let mut seed_impls_for = |local: &str, exported: &str| {
            if local == exported {
                impls.extend(
                    analysis
                        .impls
                        .iter()
                        .filter(|(_, ty, _)| ty == exported)
                        .cloned(),
                );
            }
        };
        match &binding.spec {
            ImportSpec::Bare => {}
            ImportSpec::Namespace(ns) => {
                if !binding.type_only {
                    symbols.push(Symbol {
                        name: ns.clone(),
                        type_: Type::Any,
                        kind: SymbolKind::Variable,
                        is_const: true,
                    });
                }
            }
            ImportSpec::Named(names) => {
                for (name, alias) in names {
                    let local = alias.clone().unwrap_or_else(|| name.clone());
                    if binding.type_only {
                        seed_impls_for(&local, name);
                        types.push(exported_type(analysis, name, local)?);
                        continue;
                    }
                    let exported = analysis
                        .exported_symbols
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, s)| s.clone());
                    let Some(sym) = exported else {
                        return Err(no_export(&target_readable, name));
                    };
                    symbols.push(Symbol {
                        name: local.clone(),
                        type_: sym.type_.clone(),
                        kind: sym.kind.clone(),
                        is_const: true,
                    });
                    // Importing an enum gives both a value and a type.
                    if let Type::Named(named) = &sym.type_
                        && let Some((_, entry)) =
                            analysis.exported_types.iter().find(|(n, _)| n == named)
                        && matches!(entry.kind, TypeEntryKind::Enum(_))
                    {
                        seed_impls_for(&local, named);
                        types.push((local, entry.clone()));
                    }
                }
            }
        }
    }
    Ok((symbols, types, impls))
}

fn exported_type(
    analysis: &AnalysisResult,
    name: &str,
    local: String,
) -> Result<(String, TypeEntry), XuloError> {
    match analysis
        .exported_types
        .iter()
        .find(|(n, _)| n == name)
        .cloned()
    {
        Some((_, entry)) => Ok((local, entry)),
        None => Err(no_export_type(name)),
    }
}

fn no_export_type(name: &str) -> XuloError {
    XuloError::new(
        ErrorKind::Semantic,
        format!("imported type `{name}` does not exist in the target module"),
    )
}

fn no_export(module: &dyn std::fmt::Display, name: &str) -> XuloError {
    XuloError::new(
        ErrorKind::Semantic,
        format!("module `{module}` has no export named `{name}`"),
    )
}
