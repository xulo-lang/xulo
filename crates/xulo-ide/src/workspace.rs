//! Multi-file analysis: resolve `import` statements across the files of the
//! workspace, analyze each module in dependency order (seeding dependents with
//! their imports' exports), and answer cross-module queries — most notably
//! go-to-definition across file boundaries.
//!
//! This re-implements the compiler's module loading against an *in-memory*
//! source map (the shape an editor keeps), so the analysis layer never touches
//! the compile pipeline (nor the deprecated `xulo-codegen`).

use std::collections::{HashMap, HashSet};
use std::ops::Range as ByteRange;
use std::path::{Component, Path, PathBuf};

use xulo_core::ast::{ExportItem, ImportSpec, Program, Statement, Type};
use xulo_core::error::{ErrorKind, XuloError};
use xulo_semantic::symbol_table::{Symbol, SymbolKind};
use xulo_semantic::{TypeEntry, TypeEntryKind, analyze_partial};

use crate::analysis::Analysis;
use crate::line_index::{Pos, Range};

/// Where an imported local name points: the exporting file and the exported
/// name it binds to (`None` for a namespace import, which exposes every
/// export).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportInfo {
    pub target: PathBuf,
    pub exported: Option<String>,
}

/// A resolved location: a file plus an LSP range in that file.
#[derive(Debug, Clone, PartialEq)]
pub struct Located {
    pub file: PathBuf,
    pub range: Range,
}

/// One module of the workspace: its source, its own [`Analysis`], and how its
/// `import` statements resolve.
#[derive(Debug, Clone)]
pub struct WorkspaceModule {
    pub file: PathBuf,
    pub source: String,
    pub analysis: Analysis,
    /// `(spec, type_only, target file)`. `target` is `None` when the
    /// specifier resolves to no local file (external package / unresolved):
    /// such names are checked opaquely.
    pub imports: Vec<(ImportSpec, bool, Option<PathBuf>)>,
}

/// The whole workspace: every module keyed by normalized path, with the entry
/// module remembered.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    modules: HashMap<PathBuf, WorkspaceModule>,
    entry: PathBuf,
}

impl Workspace {
    /// Load and analyze the module graph reachable from `entry` against the
    /// in-memory `sources` map (keyed by path, the document URIs an editor
    /// would send). Modules are analyzed in dependency order; a module whose
    /// own analysis fails still appears in the workspace with its error, and
    /// dependents degrade to opaque imports rather than aborting.
    pub fn open(sources: &HashMap<PathBuf, String>, entry: &Path) -> Result<Workspace, XuloError> {
        let sources: HashMap<PathBuf, String> = sources
            .iter()
            .map(|(path, src)| (normalize(path), src.clone()))
            .collect();
        let entry = normalize(entry);
        let mut builder = Builder {
            sources: &sources,
            analyzed: HashMap::new(),
            visiting: HashSet::new(),
        };
        builder.build(&entry)?;
        Ok(Workspace {
            modules: builder.analyzed,
            entry,
        })
    }

    pub fn entry(&self) -> &Path {
        &self.entry
    }

    /// Every loaded module.
    pub fn modules(&self) -> impl Iterator<Item = &WorkspaceModule> {
        self.modules.values()
    }

    /// The module at `file`, if it is part of the workspace.
    pub fn module(&self, file: &Path) -> Option<&WorkspaceModule> {
        self.modules.get(&normalize(file))
    }

    /// The per-document analysis at `file` (for document-local queries).
    pub fn analysis(&self, file: &Path) -> Option<&Analysis> {
        self.module(file).map(|m| &m.analysis)
    }

    /// Go-to-definition at `pos` in `file`, following the resolution across
    /// module boundaries: an imported name jumps into the exporting module,
    /// anything else resolves within its own file.
    pub fn go_to_definition(&self, file: &Path, pos: Pos) -> Option<Located> {
        let module = self.module(file)?;
        let analysis = &module.analysis;
        let byte = analysis.position_to_byte(pos)?;
        let result = analysis.result()?;
        let record = result.resolutions.iter().find(|r| r.span.contains(&byte));
        // A local declaration always wins: a name that shadows an import (e.g.
        // `import { foo } ...` followed by a local `let foo`) must jump to the
        // local binding, not the exporting module. Imported names carry no
        // local def, so they fall through to the import edge below.
        if let Some(def) = record.and_then(|r| r.def.as_ref()) {
            let range = analysis.line_index.span_to_range(&module.source, def)?;
            return Some(Located {
                file: file.to_path_buf(),
                range,
            });
        }
        if let Some(link) = record.and_then(|r| self.import_link(file, &r.name)) {
            return self.imported_definition(&link);
        }
        // The cursor may sit directly on a declaration (e.g. `let m ...`):
        // report the declaration itself as its own definition.
        let def = result
            .resolutions
            .iter()
            .filter_map(|r| r.def.as_ref())
            .find(|def| def.contains(&byte))?;
        let range = analysis.line_index.span_to_range(&module.source, def)?;
        Some(Located {
            file: file.to_path_buf(),
            range,
        })
    }

    /// The [`ImportInfo`] a name in `file` resolves to (the file that exports
    /// it and the exported name it binds), if the name is imported.
    fn import_link(&self, file: &Path, name: &str) -> Option<ImportInfo> {
        let module = self.module(file)?;
        for (spec, _type_only, target) in &module.imports {
            let Some(target) = target else {
                continue;
            };
            match spec {
                ImportSpec::Namespace(ns) if ns == name => {
                    return Some(ImportInfo {
                        target: target.clone(),
                        exported: None,
                    });
                }
                ImportSpec::Named(names) => {
                    for (exported, alias) in names {
                        let local = alias.clone().unwrap_or_else(|| exported.clone());
                        if local == name {
                            return Some(ImportInfo {
                                target: target.clone(),
                                exported: Some(exported.clone()),
                            });
                        }
                    }
                }
                ImportSpec::Bare | ImportSpec::Namespace(_) => {}
            }
        }
        None
    }

    /// The definition of an imported name inside its exporting module.
    fn imported_definition(&self, link: &ImportInfo) -> Option<Located> {
        let target = self.module(&link.target)?;
        let span = match &link.exported {
            Some(name) => decl_span(target.analysis.program()?, name)?,
            // A namespace import anchors at the start of the target file.
            None => 0..0,
        };
        let range = target
            .analysis
            .line_index
            .span_to_range(&target.source, &span)?;
        Some(Located {
            file: link.target.clone(),
            range,
        })
    }
}

struct Builder<'a> {
    sources: &'a HashMap<PathBuf, String>,
    analyzed: HashMap<PathBuf, WorkspaceModule>,
    /// Files on the current DFS stack, for cycle detection.
    visiting: HashSet<PathBuf>,
}

impl Builder<'_> {
    /// Parse `file`, recurse into its local imports, then analyze it in
    /// dependency order (dependents come after their dependencies, so seeds
    /// are always available).
    fn build(&mut self, file: &Path) -> Result<(), XuloError> {
        if self.analyzed.contains_key(file) {
            return Ok(());
        }
        if self.visiting.contains(file) {
            return Err(XuloError::new(
                ErrorKind::Semantic,
                format!("circular import involving {}", file.display()),
            ));
        }
        let source = self.sources.get(file).cloned().ok_or_else(|| {
            XuloError::new(
                ErrorKind::Io,
                format!("no such file in workspace: {}", file.display()),
            )
        })?;
        let tokens = xulo_lexer::tokenize(&source).map_err(|e| e.with_file(file.to_path_buf()))?;
        let program =
            xulo_parser::parse_program(&tokens).map_err(|e| e.with_file(file.to_path_buf()))?;

        let mut imports = Vec::new();
        self.visiting.insert(file.to_path_buf());
        for statement in &program.statements {
            if let Statement::Import(import) = statement {
                let target = resolve_local(self.sources, file, &import.source);
                if let Some(target) = &target {
                    self.build(target)?;
                }
                imports.push((import.spec.clone(), import.type_only, target));
            }
        }
        self.visiting.remove(file);

        let (symbols, types, impls) = seed(self.analyzed(), file, &imports);
        // Use the partial analyzer so a single failing statement never blanks
        // out the whole document: resolutions/types gathered elsewhere still
        // feed hover / go-to-definition while the error is reported.
        let (result, error) = analyze_partial(&program, &symbols, &types, &impls);
        let analysis = Analysis {
            source: source.clone(),
            program: Some(program),
            result: Some(result),
            error,
            line_index: crate::line_index::LineIndex::new(&source),
        };
        self.analyzed.insert(
            file.to_path_buf(),
            WorkspaceModule {
                file: file.to_path_buf(),
                source,
                analysis,
                imports,
            },
        );
        Ok(())
    }

    fn analyzed(&self) -> &HashMap<PathBuf, WorkspaceModule> {
        &self.analyzed
    }
}

/// Gather the symbols/types/impl registrations an importing module's `import`
/// statements pull in from its (already analyzed) dependencies. A dependency
/// whose own analysis failed contributes nothing; the importing module then
/// checks those names opaquely instead of failing the whole workspace.
type ImportSeed = (
    Vec<Symbol>,
    Vec<(String, TypeEntry)>,
    Vec<(String, String, String)>,
);

fn seed(
    analyzed: &HashMap<PathBuf, WorkspaceModule>,
    _file: &Path,
    imports: &[(ImportSpec, bool, Option<PathBuf>)],
) -> ImportSeed {
    let mut symbols: Vec<Symbol> = Vec::new();
    let mut types: Vec<(String, TypeEntry)> = Vec::new();
    let impls: Vec<(String, String, String)> = Vec::new();
    for (spec, type_only, target) in imports {
        let Some(target_path) = target else {
            continue;
        };
        let Some(module) = analyzed.get(target_path) else {
            continue;
        };
        let Some(result) = &module.analysis.result else {
            continue;
        };
        match spec {
            ImportSpec::Bare => {}
            ImportSpec::Namespace(ns) => {
                if !type_only {
                    symbols.push(Symbol {
                        name: ns.clone(),
                        type_: Type::Any,
                        kind: SymbolKind::Variable,
                        is_const: true,
                        is_mutable: false,
                    });
                }
            }
            ImportSpec::Named(names) => {
                for (exported, alias) in names {
                    let local = alias.clone().unwrap_or_else(|| exported.clone());
                    if *type_only {
                        types.push(exported_type(result, exported, local));
                        continue;
                    }
                    let Some(sym) = result
                        .exported_symbols
                        .iter()
                        .find(|(n, _)| n == exported)
                        .map(|(_, s)| s.clone())
                    else {
                        // The target exports no such value: the name stays
                        // opaque so the importing module reports its own
                        // diagnostic rather than the loader aborting.
                        continue;
                    };
                    symbols.push(Symbol {
                        name: local.clone(),
                        type_: sym.type_.clone(),
                        kind: sym.kind.clone(),
                        is_const: true,
                        is_mutable: false,
                    });
                    if let Type::Named(named) = &sym.type_
                        && let Some((_, entry)) =
                            result.exported_types.iter().find(|(n, _)| n == named)
                        && matches!(entry.kind, TypeEntryKind::Enum(_))
                    {
                        types.push((local, entry.clone()));
                    }
                }
            }
        }
    }
    (symbols, types, impls)
}

fn exported_type(
    result: &xulo_semantic::AnalysisResult,
    name: &str,
    local: String,
) -> (String, TypeEntry) {
    let entry = result
        .exported_types
        .iter()
        .find(|(n, _)| n == name)
        .cloned()
        .map(|(_, entry)| entry)
        .unwrap_or_else(|| TypeEntry {
            type_params: Vec::new(),
            kind: TypeEntryKind::Alias(Type::Any),
        });
    (local, entry)
}

/// Try to resolve an import specifier against a local file in the workspace:
/// `./b`, `./b.xulo`, or `./b/index.xulo`.
fn resolve_local(
    sources: &HashMap<PathBuf, String>,
    importer: &Path,
    specifier: &str,
) -> Option<PathBuf> {
    let base = importer.parent().unwrap_or_else(|| Path::new(""));
    let joined = base.join(specifier);
    let mut candidates = Vec::new();
    candidates.push(joined.clone());
    candidates.push(PathBuf::from(format!("{}.xulo", joined.display())));
    candidates.push(joined.join("index.xulo"));
    candidates.into_iter().find_map(|candidate| {
        let normalized = normalize(&candidate);
        sources.contains_key(&normalized).then_some(normalized)
    })
}

/// The byte span of the top-level declaration of `name` in `program` (its
/// `name_span`), if any.
fn decl_span(program: &Program, name: &str) -> Option<ByteRange<usize>> {
    for statement in &program.statements {
        match statement {
            Statement::Fn(f) if f.name == name => return Some(f.name_span.clone()),
            Statement::Export(export) => match &export.item {
                ExportItem::Fn(f) if f.name == name => return Some(f.name_span.clone()),
                ExportItem::Let(b) if b.name == name => return Some(b.name_span.clone()),
                ExportItem::Type(a) if a.name == name => return Some(a.name_span.clone()),
                ExportItem::Enum(e) if e.name == name => return Some(e.name_span.clone()),
                ExportItem::Trait(t) if t.name == name => return Some(t.name_span.clone()),
                ExportItem::Names(_) => {}
                _ => {}
            },
            Statement::Let(b) if b.name == name => return Some(b.name_span.clone()),
            Statement::TypeAlias(a) if a.name == name => return Some(a.name_span.clone()),
            Statement::Enum(e) if e.name == name => return Some(e.name_span.clone()),
            Statement::Trait(t) if t.name == name => return Some(t.name_span.clone()),
            Statement::State(s) if s.binding.name == name => {
                return Some(s.binding.name_span.clone());
            }
            Statement::Environment(e) if e.name == name => return Some(e.name_span.clone()),
            _ => {}
        }
    }
    None
}

/// Lexically normalize a path (resolve `.` and `..` without touching the
/// filesystem), so in-memory document keys compare consistently.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}
