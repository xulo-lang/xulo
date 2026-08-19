// Xulo Language Server client (dependency-free, hand-rolled JSON-RPC over
// stdio). The server binary is `xulo-analyzer` from this repo's `crates/xulo-analyzer`.
//
// To run from source: build with `cargo build -p xulo-analyzer` and either add
// `target/debug` to PATH or set `xulo.server.path` to the binary.

const vscode = require('vscode');
const { spawn } = require('child_process');
const path = require('path');

let diagnostics;
let output;
let client = null;

const SEMANTIC_LEGEND = {
  tokenTypes: [
    'variable',
    'parameter',
    'function',
    'method',
    'property',
    'constant',
    'type',
    'enum',
    'interface',
    'enumMember',
    'class',
  ],
  tokenModifiers: ['declaration'],
};

function activate(context) {
  diagnostics = vscode.languages.createDiagnosticCollection('xulo');
  output = vscode.window.createOutputChannel('Xulo Language Server');
  context.subscriptions.push(diagnostics, output);

  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument((doc) => client && doc.languageId === 'xulo' && client.didOpen(doc)),
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (!client || event.document.languageId !== 'xulo') return;
      client.didChange(event.document, event.contentChanges);
    }),
    vscode.workspace.onDidCloseTextDocument((doc) => client && doc.languageId === 'xulo' && client.didClose(doc)),
    vscode.commands.registerCommand('xulo.restartServer', () => restart()),
    vscode.languages.registerDefinitionProvider('xulo', {
      provideDefinition: (doc, pos) => request('textDocument/definition', positionParams(doc, pos)),
    }),
    vscode.languages.registerHoverProvider('xulo', {
      provideHover: (doc, pos) => request('textDocument/hover', positionParams(doc, pos)),
    }),
    vscode.languages.registerReferenceProvider('xulo', {
      provideReferences: (doc, pos) =>
        request('textDocument/references', {
          textDocument: { uri: doc.uri.toString() },
          position: lspPos(pos),
          context: { includeDeclaration: true },
        }),
    }),
    vscode.languages.registerDocumentSymbolProvider('xulo', {
      provideDocumentSymbols: (doc) => request('textDocument/documentSymbol', { textDocument: { uri: doc.uri.toString() } }),
    }),
    vscode.languages.registerDocumentSemanticTokensProvider('xulo', {
      provideDocumentSemanticTokens: (doc) =>
        request('textDocument/semanticTokens/full', { textDocument: { uri: doc.uri.toString() } })
          .then((result) => (result ? new vscode.SemanticTokens(new Uint32Array(result.data), SEMANTIC_LEGEND) : null)),
    }, SEMANTIC_LEGEND),
    vscode.languages.registerDocumentFormattingEditProvider('xulo', {
      provideDocumentFormattingEdits: (doc) =>
        request('textDocument/formatting', {
          textDocument: { uri: doc.uri.toString() },
          options: { tabSize: 2, insertSpaces: true },
        }),
    })
  );

  ensureClient();
}

function deactivate() {
  if (client) client.stop();
  client = null;
}

async function restart() {
  if (client) client.stop();
  client = null;
  ensureClient();
  for (const doc of vscode.workspace.textDocuments) {
    if (doc.languageId === 'xulo') client.didOpen(doc);
  }
}

function ensureClient() {
  if (client) return;
  client = new Client();
  client.onDiagnostics((uri, lspDiags) => {
    const docUri = vscode.Uri.parse(uri);
    diagnostics.set(docUri, lspDiags.map(toVscodeDiagnostic));
  });
  client.start();
  for (const doc of vscode.workspace.textDocuments) {
    if (doc.languageId === 'xulo') client.didOpen(doc);
  }
}

function positionParams(doc, pos) {
  return { textDocument: { uri: doc.uri.toString() }, position: lspPos(pos) };
}

function lspPos(pos) {
  return { line: pos.line, character: pos.character };
}

async function request(method, params) {
  if (!client) return null;
  return client.request(method, params).then(mapResponse(method), () => null);
}

function mapResponse(method) {
  if (method === 'textDocument/definition') {
    return (loc) => (loc ? new vscode.Location(vscode.Uri.parse(loc.uri), toVscodeRange(loc.range)) : null);
  }
  if (method === 'textDocument/hover') {
    return (h) => (h ? new vscode.Hover(toMarkdown(h.contents), toVscodeRange(h.range)) : null);
  }
  if (method === 'textDocument/references') {
    return (locs) => (locs || []).map((loc) => new vscode.Location(vscode.Uri.parse(loc.uri), toVscodeRange(loc.range)));
  }
  if (method === 'textDocument/documentSymbol') {
    return (symbols) => (symbols || []).map(toVscodeSymbol);
  }
  return (v) => v;
}

function toMarkdown(contents) {
  if (contents && contents.kind === 'markdown') {
    return new vscode.MarkdownString(contents.value);
  }
  return new vscode.MarkdownString(String(contents || ''));
}

function toVscodeRange(range) {
  return new vscode.Range(range.start.line, range.start.character, range.end.line, range.end.character);
}

function toVscodeDiagnostic(d) {
  const severityMap = [vscode.DiagnosticSeverity.Error, vscode.DiagnosticSeverity.Warning, vscode.DiagnosticSeverity.Information, vscode.DiagnosticSeverity.Hint];
  return new vscode.Diagnostic(toVscodeRange(d.range), d.message, severityMap[d.severity - 1] ?? vscode.DiagnosticSeverity.Error);
}

function toVscodeSymbol(s) {
  const kinds = {
    2: vscode.SymbolKind.Function,
    4: vscode.SymbolKind.Method,
    13: vscode.SymbolKind.Variable,
    14: vscode.SymbolKind.Constant,
    5: vscode.SymbolKind.Property,
    10: vscode.SymbolKind.Interface,
    8: vscode.SymbolKind.Struct,
    18: vscode.SymbolKind.Enum,
    20: vscode.SymbolKind.EnumMember,
    5: vscode.SymbolKind.Class,
  };
  const symbol = new vscode.DocumentSymbol(
    s.name,
    s.detail || '',
    kinds[s.kind] ?? vscode.SymbolKind.Variable,
    toVscodeRange(s.range),
    toVscodeRange(s.selectionRange)
  );
  symbol.children = (s.children || []).map(toVscodeSymbol);
  return symbol;
}

class Client {
  constructor() {
    this.child = null;
    this.buffer = Buffer.alloc(0);
    this.nextId = 0;
    this.pending = new Map();
    this.diagHandler = null;
    this.ready = null;
  }

  onDiagnostics(handler) {
    this.diagHandler = handler;
  }

  start() {
    const binary = vscode.workspace.getConfiguration('xulo').get('server.path') || process.env.XULO_LSP || 'xulo-analyzer';
    this.child = spawn(binary, [], { stdio: ['pipe', 'pipe', 'pipe'] });
    this.child.stderr.on('data', (data) => output.append(data.toString()));
    this.child.on('error', (err) => {
      output.appendLine(`failed to start ${binary}: ${err.message}`);
      vscode.window.showErrorMessage(`xulo-analyzer could not be started: ${err.message}`);
    });
    this.child.stdout.on('data', (data) => this.onData(data));
    this.child.on('exit', (code, signal) => {
      output.appendLine(`xulo-analyzer exited (code=${code} signal=${signal})`);
      client = null;
    });

    this.ready = this.request('initialize', {
      processId: process.pid,
      rootUri: rootUri(),
      capabilities: { textDocument: { synchronization: { didSave: false, incremental: true } } },
    }).then(() => {
      this.notify('initialized', {});
    });
  }

  stop() {
    if (!this.child) return;
    this.notify('exit', null);
    try {
      this.child.kill();
    } catch (_) { /* already gone */ }
  }

  didOpen(doc) {
    this.ready.then(() => {
      this.notify('textDocument/didOpen', {
        textDocument: { uri: doc.uri.toString(), languageId: 'xulo', version: doc.version, text: doc.getText() },
      });
    });
  }

  didChange(doc, contentChanges) {
    const changes = contentChanges.map((change) => ({
      range: change.range ? { start: lspPos(change.range.start), end: lspPos(change.range.end) } : undefined,
      text: change.text,
    }));
    this.ready.then(() => {
      this.notify('textDocument/didChange', {
        textDocument: { uri: doc.uri.toString(), version: doc.version },
        contentChanges: changes,
      });
    });
  }

  didClose(doc) {
    this.ready.then(() => {
      this.notify('textDocument/didClose', { textDocument: { uri: doc.uri.toString() } });
    });
  }

  request(method, params) {
    const id = ++this.nextId;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ jsonrpc: '2.0', id, method, params });
    });
  }

  notify(method, params) {
    this.send({ jsonrpc: '2.0', method, params });
  }

  send(message) {
    const body = Buffer.from(JSON.stringify(message));
    this.child.stdin.write(Buffer.from(`Content-Length: ${body.length}\r\n\r\n`));
    this.child.stdin.write(body);
  }

  onData(data) {
    this.buffer = Buffer.concat([this.buffer, data]);
    for (;;) {
      const headerEnd = this.buffer.indexOf('\r\n\r\n');
      if (headerEnd === -1) return;
      const header = this.buffer.slice(0, headerEnd).toString();
      const match = /Content-Length: (\d+)/i.exec(header);
      if (!match) {
        this.buffer = this.buffer.slice(headerEnd + 4);
        continue;
      }
      const length = Number(match[1]);
      const bodyStart = headerEnd + 4;
      if (this.buffer.length < bodyStart + length) return;
      const body = this.buffer.slice(bodyStart, bodyStart + length).toString();
      this.buffer = this.buffer.slice(bodyStart + length);
      this.onMessage(JSON.parse(body));
    }
  }

  onMessage(message) {
    if (message.method === 'textDocument/publishDiagnostics') {
      if (this.diagHandler) this.diagHandler(message.params.uri, message.params.diagnostics || []);
      return;
    }
    if (message.id !== undefined) {
      const entry = this.pending.get(message.id);
      if (!entry) return;
      this.pending.delete(message.id);
      if (message.error) {
        entry.reject(new Error(`${message.error.code}: ${message.error.message}`));
      } else {
        entry.resolve(message.result);
      }
    }
  }
}

function rootUri() {
  const folder = vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0];
  return folder ? folder.uri.toString() : null;
}

module.exports = { activate, deactivate };