//! Multi-file module loading and bundling.
//!
//! `import` statements with a specifier that resolves to a local `.xulo` file
//! (relative paths, or a bare name that exists next to the importer) are loaded,
//! semantically analyzed in dependency order, and bundled into a single
//! JavaScript file. Any other specifier is treated as an external package and
//! emitted verbatim as an ES-module `import` at the top of the bundle.
//!
//! There is no runtime module loader: each bundled module becomes an IIFE that
//! returns its exports object, and importers read bindings from that object.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::{ExportItem, ImportSpec, ImportStmt, Program, Statement, Type};
use crate::error::{ErrorKind, XuloError};
use crate::semantic::{analyze_with, AnalysisResult, TypeEntry, TypeEntryKind};
use crate::semantic::symbol_table::{Symbol, SymbolKind};

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

/// Load, analyze in dependency order, and bundle the module graph reachable
/// from `entry`.
pub fn compile_file(entry: &Path) -> Result<String, XuloError> {
    let mut loaded = load(entry)?;
    analyze(&mut loaded)?;
    bundle(&loaded)
}

/// Resolve the transitive local imports of `entry`.
pub fn load(entry: &Path) -> Result<LoadedModules, XuloError> {
    let entry = entry
        .canonicalize()
        .map_err(|e| XuloError::new(ErrorKind::Io, format!("cannot resolve {}: {e}", entry.display())))?;
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
            XuloError::new(ErrorKind::Io, format!("cannot resolve {}: {e}", file.display()))
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
            XuloError::new(ErrorKind::Io, format!("cannot read {}: {e}", file.display()))
        })?;
        let tokens = crate::lexer::tokenize(&source).map_err(|e| e.with_file(file.clone()))?;
        let program =
            crate::parser::parse_program(&tokens).map_err(|e| e.with_file(file.clone()))?;

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

/// Does this module declare a `main` function (directly or via `export`)?
fn program_has_main(program: &Program) -> bool {
    fn fn_named(f: &crate::ast::FnDef) -> bool {
        f.name == "main"
    }
    program.statements.iter().any(|s| match s {
        Statement::Fn(f) => fn_named(f),
        Statement::Export(export) => match &export.item {
            ExportItem::Fn(f) => fn_named(f),
            ExportItem::Default(inner) => matches!(inner.as_ref(), ExportItem::Fn(f) if fn_named(f)),
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
    candidates
        .into_iter()
        .find(|p| p.is_file())
}

/// Analyze every module in dependency order, seeding each one with the
/// symbols and types its imports pull in. Rejects imports of names the target
/// does not export.
pub fn analyze(loaded: &mut LoadedModules) -> Result<(), XuloError> {
    let modules = &mut loaded.modules;
    let count = modules.len();
    for idx in 0..count {
        let (symbols, types) = collect_imports(modules, idx)?;
        let result = analyze_with(&modules[idx].program, &symbols, &types)
            .map_err(|e| e.with_message_prefix(format!("{}: ", modules[idx].file.display())))?;
        modules[idx].analysis = Some(result);
    }
    Ok(())
}

fn collect_imports(
    modules: &[Module],
    idx: usize,
) -> Result<(Vec<Symbol>, Vec<(String, TypeEntry)>), XuloError> {
    let mut symbols: Vec<Symbol> = Vec::new();
    let mut types: Vec<(String, TypeEntry)> = Vec::new();
    for binding in &modules[idx].imports {
        let target = &modules[binding.target];
        let analysis = target
            .analysis
            .as_ref()
            .expect("dependencies are analyzed before dependents");
        let target_readable = target.file.display();
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
                        types.push(exported_type(analysis, name, local));
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
                        && let Some((_, entry)) = analysis
                            .exported_types
                            .iter()
                            .find(|(n, _)| n == named)
                        && matches!(entry.kind, TypeEntryKind::Enum(_))
                    {
                        types.push((local, entry.clone()));
                    }
                }
            }
            ImportSpec::Default(name) => {
                if binding.type_only {
                    // No default *type* exports exist; treat as opaque.
                    types.push((name.clone(), TypeEntry {
                        type_params: Vec::new(),
                        kind: TypeEntryKind::Alias(Type::Any),
                    }));
                    continue;
                }
                let Some(default_name) = &analysis.default else {
                    return Err(XuloError::new(
                        ErrorKind::Semantic,
                        format!("module `{}` has no default export", target_readable),
                    ));
                };
                let sym = analysis
                    .exported_symbols
                    .iter()
                    .find(|(n, _)| n == default_name)
                    .map(|(_, s)| s.clone())
                    .unwrap_or_else(|| Symbol {
                        name: default_name.clone(),
                        type_: Type::Any,
                        kind: SymbolKind::Variable,
                        is_const: true,
                    });
                symbols.push(Symbol {
                    name: name.clone(),
                    type_: sym.type_.clone(),
                    kind: sym.kind.clone(),
                    is_const: true,
                });
            }
        }
    }
    Ok((symbols, types))
}

fn exported_type(analysis: &AnalysisResult, name: &str, local: String) -> (String, TypeEntry) {
    match analysis
        .exported_types
        .iter()
        .find(|(n, _)| n == name)
        .cloned()
    {
        Some((_, entry)) => (local, entry),
        None => (
            local,
            TypeEntry {
                type_params: Vec::new(),
                kind: TypeEntryKind::Alias(Type::Any),
            },
        ),
    }
}

fn no_export(module: &dyn std::fmt::Display, name: &str) -> XuloError {
    XuloError::new(
        ErrorKind::Semantic,
        format!("module `{module}` has no export named `{name}`"),
    )
}

/// Emit one JavaScript file for the whole bundle: external imports as ESM at
/// the top, then each module as an IIFE returning its exports. The entry
/// module's `main()` runs when the file is loaded.
fn bundle(loaded: &LoadedModules) -> Result<String, XuloError> {
    let mut out = String::new();
    for imp in &loaded.external_imports {
        match &imp.spec {
            ImportSpec::Bare => out.push_str(&format!("import {:?};\n", imp.source)),
            ImportSpec::Namespace(ns) => {
                out.push_str(&format!("import * as {ns} from {:?};\n", imp.source))
            }
            ImportSpec::Named(names) => {
                let parts = names
                    .iter()
                    .map(|(name, alias)| match alias {
                        Some(a) => format!("{name} as {a}"),
                        None => name.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!("import {{ {parts} }} from {:?};\n", imp.source));
            }
            ImportSpec::Default(name) => {
                out.push_str(&format!("import {name} from {:?};\n", imp.source))
            }
        }
    }

    let entry = loaded.entry;
    for (idx, module) in loaded.modules.iter().enumerate() {
        let mut cg = crate::codegen::javascript::Javascript::new();
        // Imported functions may be called with named arguments.
        for binding in &module.imports {
            if binding.type_only {
                continue;
            }
            let target = &loaded.modules[binding.target];
            let analysis = target.analysis.as_ref().expect("analyzed");
            match &binding.spec {
                ImportSpec::Named(names) => {
                    for (name, alias) in names {
                        let local = alias.clone().unwrap_or_else(|| name.clone());
                        if let Some((_, sym)) = analysis
                            .exported_symbols
                            .iter()
                            .find(|(n, _)| n == name)
                        {
                            if let SymbolKind::Function(_, params, _) = &sym.kind {
                                cg.register_fn_params(
                                    local.clone(),
                                    params.iter().map(|p| p.name.clone()).collect(),
                                );
                            }
                        }
                    }
                }
                ImportSpec::Default(name) => {
                    if let Some(default_name) = &analysis.default
                        && let Some((_, sym)) = analysis
                            .exported_symbols
                            .iter()
                            .find(|(n, _)| n == default_name)
                        && let SymbolKind::Function(_, params, _) = &sym.kind
                    {
                        cg.register_fn_params(
                            name.clone(),
                            params.iter().map(|p| p.name.clone()).collect(),
                        );
                    }
                }
                _ => {}
            }
        }

        out.push_str(&format!("const __mod{idx} = (() => {{\n"));
        // One destructuring per target module, so the same name is never
        // bound twice (`import { Color }` + `import type { Color }`) and a
        // type-only import of an enum still gets its runtime value (so
        // `Color::Red` works in value position).
        let mut bound: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut per_target: std::collections::HashMap<usize, Vec<String>> =
            std::collections::HashMap::new();
        for binding in &module.imports {
            let target = &loaded.modules[binding.target];
            let analysis = target.analysis.as_ref().expect("analyzed");
            let is_runtime_value = |name: &str| {
                analysis.exported_symbols.iter().any(|(n, _)| n == name)
            };
            match &binding.spec {
                ImportSpec::Bare => {}
                ImportSpec::Namespace(ns) => {
                    if !binding.type_only && !bound.contains(ns) {
                        out.push_str(&format!("    const {ns} = __mod{};\n", binding.target));
                        bound.insert(ns.clone());
                    }
                }
                ImportSpec::Named(names) => {
                    let entries = per_target.entry(binding.target).or_default();
                    for (name, alias) in names {
                        let local = alias.clone().unwrap_or_else(|| name.clone());
                        if binding.type_only {
                            // Only enums/types with a runtime value survive.
                            if !is_runtime_value(name) || bound.contains(&local) {
                                continue;
                            }
                        } else if bound.contains(&local) {
                            continue;
                        }
                        bound.insert(local.clone());
                        entries.push(match alias {
                            Some(a) => format!("{name}: {a}"),
                            None => name.clone(),
                        });
                    }
                }
                ImportSpec::Default(name) => {
                    if !binding.type_only && !bound.contains(name) {
                        out.push_str(&format!(
                            "    const {name} = __mod{}.default;\n",
                            binding.target
                        ));
                        bound.insert(name.clone());
                    }
                }
            }
        }
        let mut targets: Vec<usize> = per_target.keys().copied().collect();
        targets.sort_unstable();
        for target in targets {
            let names = per_target.get(&target).expect("key").join(", ");
            out.push_str(&format!("    const {{ {names} }} = __mod{target};\n"));
        }
        cg.emit_module_body(&module.program)?;
        out.push_str(&cg.finish());
        if idx == entry && module.has_main {
            if crate::codegen::javascript::main_returns_component(&module.program) {
                out.push_str("    const __xulo_main = main();\n");
                out.push_str("    if (typeof __xulo_mount === \"function\") __xulo_mount(__xulo_main);\n");
            } else {
                out.push_str("    main();\n");
            }
        }
        let analysis = module.analysis.as_ref().expect("analyzed");
        let mut exports: Vec<String> = analysis
            .exported_symbols
            .iter()
            .map(|(name, _)| format!("{name}: {name}"))
            .collect();
        if let Some(def) = &analysis.default {
            exports.push(format!("default: {def}"));
        }
        let exports = if exports.is_empty() {
            "{}".to_string()
        } else {
            format!("{{ {} }}", exports.join(", "))
        };
        out.push_str(&format!("    return {exports};\n}})();\n"));
    }
    Ok(out)
}