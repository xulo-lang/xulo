// Xulo Language Server client (dependency-free, hand-rolled JSON-RPC over
// stdio). The server binary is `xulo-analyzer` from this repo's
// `crates/xulo-analyzer`.
//
// To run from source: build with `cargo build -p xulo-analyzer` and either add
// `target/debug` to PATH or set `xulo.server.path` to the binary.

const vscode = require('vscode');
const { spawn } = require('child_process');

let diagnostics;
let output;
let status;
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

// Request budgets in milliseconds. A hover must never stall the UI, while a
// whole-document rewrite over a large file legitimately takes longer.
const TIMEOUTS = {
  'textDocument/formatting': 20000,
  'textDocument/semanticTokens/full': 10000,
  default: 3000,
};

function activate(context) {
  diagnostics = vscode.languages.createDiagnosticCollection('xulo');
  output = vscode.window.createOutputChannel('Xulo Language Server');
  status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
  status.command = 'xulo.restartServer';
  status.text = '$(loading~spin) Xulo';
  status.tooltip = 'Xulo language server: starting…';
  status.show();
  context.subscriptions.push(diagnostics, output, status);

  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument((doc) => client && doc.languageId === 'xulo' && client.didOpen(doc)),
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (!client || event.document.languageId !== 'xulo') return;
      client.didChange(event.document, event.contentChanges);
    }),
    vscode.workspace.onDidCloseTextDocument((doc) => {
      // Diagnostics are a per-URI collection: without this they outlive the
      // buffer and stay on the problems panel forever.
      if (doc.languageId === 'xulo') diagnostics.delete(doc.uri);
      if (client) client.didClose(doc);
    }),
    vscode.commands.registerCommand('xulo.restartServer', () => restart()),
    vscode.commands.registerCommand('xulo.showOutput', () => output.show()),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration('xulo.server.path')) restart();
    }),
    vscode.languages.registerDefinitionProvider('xulo', {
      provideDefinition: (doc, pos) => request('textDocument/definition', positionParams(doc, pos)),
    }),
    vscode.languages.registerHoverProvider('xulo', {
      provideHover: (doc, pos) => request('textDocument/hover', positionParams(doc, pos)),
    }),
    vscode.languages.registerReferenceProvider('xulo', {
      provideReferences: (doc, pos, _token, context) =>
        request('textDocument/references', {
          textDocument: { uri: doc.uri.toString() },
          position: lspPos(pos),
          context: { includeDeclaration: context ? context.includeDeclaration !== false : true },
        }),
    }),
    vscode.languages.registerDocumentSymbolProvider('xulo', {
      provideDocumentSymbols: (doc) => request('textDocument/documentSymbol', { textDocument: { uri: doc.uri.toString() } }),
    }),
    vscode.languages.registerDocumentSemanticTokensProvider(
      'xulo',
      {
        provideDocumentSemanticTokens: (doc) =>
          request('textDocument/semanticTokens/full', { textDocument: { uri: doc.uri.toString() } }).then((result) =>
            result ? new vscode.SemanticTokens(new Uint32Array(result.data), SEMANTIC_LEGEND) : null
          ),
      },
      SEMANTIC_LEGEND
    ),
    vscode.languages.registerDocumentFormattingEditProvider('xulo', {
      provideDocumentFormattingEdits: (doc, _range, options) =>
        request('textDocument/formatting', {
          textDocument: { uri: doc.uri.toString() },
          options: {
            tabSize: options.tabSize,
            insertSpaces: options.insertSpaces,
            trimTrailingWhitespace: true,
          },
        }),
    })
  );

  ensureClient();
}

function deactivate() {
  if (!client) return undefined;
  const stopped = client.stop();
  client = null;
  return stopped;
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
  client.onDiagnostics((uri, lspDiags) => diagnostics.set(vscode.Uri.parse(uri), lspDiags.map(toVscodeDiagnostic)));
  client.onStateChange((state, detail) => {
    if (state === 'running') {
      status.text = '$(check) Xulo';
      status.tooltip = 'Xulo language server: running';
    } else if (state === 'starting') {
      status.text = '$(loading~spin) Xulo';
      status.tooltip = 'Xulo language server: starting…';
    } else {
      status.text = '$(error) Xulo';
      status.tooltip = `Xulo language server: ${detail || 'stopped'}`;
    }
    status.show();
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

function request(method, params) {
  if (!client) return Promise.resolve(null);
  return client.request(method, params).then(mapResponse(method), () => null);
}

function mapResponse(method) {
  if (method === 'textDocument/definition') {
    return (loc) => (loc ? new vscode.Location(vscode.Uri.parse(loc.uri), toVscodeRange(loc.range)) : null);
  }
  if (method === 'textDocument/hover') {
    return (h) => (h ? new vscode.Hover(toMarkdown(h.contents), h.range && toVscodeRange(h.range)) : null);
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
  const severityMap = [
    vscode.DiagnosticSeverity.Error,
    vscode.DiagnosticSeverity.Warning,
    vscode.DiagnosticSeverity.Information,
    vscode.DiagnosticSeverity.Hint,
  ];
  const diag = new vscode.Diagnostic(
    toVscodeRange(d.range),
    d.message,
    severityMap[d.severity - 1] ?? vscode.DiagnosticSeverity.Error
  );
  diag.source = d.source || 'xulo';
  if (d.code !== undefined && d.code !== null) diag.code = d.code;
  if (Array.isArray(d.tags)) diag.tags = d.tags;
  return diag;
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
    6: vscode.SymbolKind.Class,
    9: vscode.SymbolKind.Field,
    23: vscode.SymbolKind.Event,
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
    this.stateHandler = null;
    this.ready = null;
    this.stopping = false;
    this._lastExitAt = 0;
  }

  onDiagnostics(handler) {
    this.diagHandler = handler;
  }

  onStateChange(handler) {
    this.stateHandler = handler;
  }

  setState(state, detail) {
    if (this.stateHandler) this.stateHandler(state, detail);
  }

  traceMode() {
    return vscode.workspace.getConfiguration('xulo').get('trace.server') || 'off';
  }

  trace(direction, message) {
    const mode = this.traceMode();
    if (mode === 'off') return;
    const body = mode === 'verbose' ? JSON.stringify(message) : `${message.method || 'response'} ${message.id ?? ''}`;
    output.appendLine(`${direction} ${body}`);
  }

  start() {
    this.stopping = false;
    const configured = resolveServerPath(vscode.workspace.getConfiguration('xulo').get('server.path'));
    // Precedence: the setting, then $XULO_LSP, then `xulo-analyzer` on PATH.
    // An empty setting means "fall through" so $XULO_LSP is reachable.
    const binary = configured || process.env.XULO_LSP || 'xulo-analyzer';
    this.setState('starting');
    this.child = spawn(binary, [], { stdio: ['pipe', 'pipe', 'pipe'] });
    this.child.stderr.on('data', (data) => output.append(data.toString()));
    this.child.on('error', (err) => {
      output.appendLine(`failed to start ${binary}: ${err.message}`);
      vscode.window.showErrorMessage(
        `xulo-analyzer could not be started (${err.message}). Set xulo.server.path to target/debug/xulo-analyzer or add it to PATH.`
      );
      this.failPending('language server failed to start');
      this.child = null;
      this.setState('stopped', err.message);
      client = null;
    });
    this.child.stdout.on('data', (data) => this.onData(data));
    this.child.on('exit', (code, signal) => {
      output.appendLine(`xulo-analyzer exited (code=${code} signal=${signal})`);
      this.failPending('language server exited');
      this.child = null;
      this.setState('stopped', `exited (code=${code} signal=${signal})`);
      client = null;
      // Unexpected death (not an explicit stop/restart): bring the server back
      // so hover / definition keep working without a manual restart. Bail out
      // if the server keeps dying at startup (wrong binary etc.) to avoid a
      // restart hot-loop.
      if (!this.stopping && code !== 0) {
        const now = Date.now();
        if (now - this._lastExitAt < 3000) {
          output.appendLine('xulo-analyzer keeps exiting; not auto-restarting (check xulo.server.path)');
        } else {
          this._lastExitAt = now;
          setTimeout(() => {
            if (!client) ensureClient();
          }, 500);
        }
      }
    });

    this.ready = this.request('initialize', {
      processId: process.pid,
      clientInfo: { name: 'vscode-xulo', version: require('./package.json').version },
      rootUri: rootUri(),
      workspaceFolders: workspaceFolders(),
      capabilities: clientCapabilities(),
    }).then(
      () => {
        this.notify('initialized', {});
        this.setState('running');
      },
      () => {
        // The server never became ready (spawn error / timeout / early exit).
        // didOpen/didChange keep queuing on `ready`, which now resolves; their
        // notifications are no-ops because `send` guards on a live child.
        this.setState('stopped', 'initialize failed');
      }
    );
  }

  stop() {
    this.stopping = true;
    let stopped;
    if (this.child) {
      this.notify('exit', null);
      try {
        this.child.kill();
      } catch (_) {
        /* already gone */
      }
      this.child = null;
      stopped = Promise.resolve();
    }
    this.failPending('language server stopped');
    return stopped;
  }

  failPending(reason) {
    for (const entry of this.pending.values()) {
      if (entry.timer) clearTimeout(entry.timer);
      entry.reject(new Error(reason));
    }
    this.pending.clear();
  }

  didOpen(doc) {
    this.ready.then(() =>
      this.notify('textDocument/didOpen', {
        textDocument: { uri: doc.uri.toString(), languageId: 'xulo', version: doc.version, text: doc.getText() },
      })
    );
  }

  didChange(doc, contentChanges) {
    const changes = contentChanges.map((change) => ({
      range: change.range ? { start: lspPos(change.range.start), end: lspPos(change.range.end) } : undefined,
      text: change.text,
    }));
    this.ready.then(() =>
      this.notify('textDocument/didChange', {
        textDocument: { uri: doc.uri.toString(), version: doc.version },
        contentChanges: changes,
      })
    );
  }

  didClose(doc) {
    this.ready.then(() => this.notify('textDocument/didClose', { textDocument: { uri: doc.uri.toString() } }));
  }

  request(method, params) {
    const id = ++this.nextId;
    return new Promise((resolve, reject) => {
      if (!this.child) {
        reject(new Error('language server not running'));
        return;
      }
      const timeout = TIMEOUTS[method] ?? TIMEOUTS.default;
      const timer = setTimeout(() => {
        if (!this.pending.delete(id)) return;
        reject(new Error(`request timed out: ${method}`));
      }, timeout);
      this.pending.set(id, { resolve, reject, timer });
      this.send({ jsonrpc: '2.0', id, method, params });
    });
  }

  notify(method, params) {
    if (!this.child) return;
    this.send({ jsonrpc: '2.0', method, params });
  }

  send(message) {
    if (!this.child) return;
    this.trace('→', message);
    const body = Buffer.from(JSON.stringify(message));
    try {
      this.child.stdin.write(Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]));
    } catch (_) {
      // The child is gone or its stdin closed: fail the matching request.
      if (message.id !== undefined && this.pending.has(message.id)) {
        const entry = this.pending.get(message.id);
        this.pending.delete(message.id);
        if (entry.timer) clearTimeout(entry.timer);
        entry.reject(new Error('language server stdin closed'));
      }
    }
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
      let message;
      try {
        message = JSON.parse(body);
      } catch (_) {
        continue;
      }
      this.trace('←', message);
      this.onMessage(message);
    }
  }

  onMessage(message) {
    if (message.method === 'textDocument/publishDiagnostics') {
      if (this.diagHandler) this.diagHandler(message.params.uri, message.params.diagnostics || []);
      return;
    }
    if (message.method && message.id !== undefined) {
      // A server -> client REQUEST. Never answering leaves the server blocked
      // on the response forever, so acknowledge everything we do not implement.
      this.handleServerRequest(message);
      return;
    }
    if (message.method) {
      this.handleServerNotification(message);
      return;
    }
    if (message.id !== undefined) {
      const entry = this.pending.get(message.id);
      if (!entry) return;
      this.pending.delete(message.id);
      clearTimeout(entry.timer);
      if (message.error) {
        entry.reject(new Error(`${message.error.code}: ${message.error.message}`));
      } else {
        entry.resolve(message.result);
      }
    }
  }

  handleServerRequest(message) {
    let result = null;
    let error = null;
    switch (message.method) {
      case 'workspace/configuration': {
        const items = (message.params && message.params.items) || [];
        result = items.map((item) => {
          const section = (item && item.section) || '';
          const dot = section.indexOf('.');
          const root = dot === -1 ? section : section.slice(0, dot);
          const rest = dot === -1 ? '' : section.slice(dot + 1);
          if (root !== 'xulo' || !rest) return null;
          const scope = item && item.scope ? vscode.Uri.parse(item.scope) : null;
          return vscode.workspace.getConfiguration('xulo', scope).get(rest);
        });
        break;
      }
      case 'workspace/applyEdit':
        result = { applied: false, failureReason: 'applyEdit is not supported by the Xulo extension' };
        break;
      case 'window/showMessageRequest': {
        const params = message.params || {};
        const actions = params.items || [];
        vscode.window
          .showMessage(params.message || '', ...(actions.length ? actions.map((i) => i.title) : []))
          .then((picked) => {
            const item = actions.find((i) => i.title === picked);
            this.respond(message.id, item || null);
          });
        return;
      }
      // Dynamic registration, progress creation, and refresh signals: accept
      // and ignore — the server advertises static capabilities only.
      case 'client/registerCapability':
      case 'client/unregisterCapability':
      case 'window/workDoneProgress/create':
      case 'workspace/semanticTokens/refresh':
      case 'workspace/inlayHint/refresh':
      case 'workspace/codeLens/refresh':
      case 'workspace/inlineValue/refresh':
        break;
      default:
        error = { code: -32601, message: `unhandled server request \`${message.method}\`` };
        break;
    }
    this.respond(message.id, result, error);
  }

  respond(id, result, error) {
    if (!this.child) return;
    const message = error ? { jsonrpc: '2.0', id, error } : { jsonrpc: '2.0', id, result };
    this.trace('→', message);
    const body = Buffer.from(JSON.stringify(message));
    try {
      this.child.stdin.write(Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]));
    } catch (_) {
      /* the child is gone */
    }
  }

  handleServerNotification(message) {
    const params = message.params || {};
    switch (message.method) {
      case 'window/logMessage':
        output.append(String(params.message || ''));
        break;
      case 'window/showMessage': {
        const text = String(params.message || '');
        if (params.type === 1) vscode.window.showErrorMessage(text);
        else if (params.type === 2) vscode.window.showWarningMessage(text);
        else if (params.type === 3 || params.type === 4) vscode.window.showInformationMessage(text);
        else output.appendLine(text);
        break;
      }
      case 'telemetry/event':
      case '$/progress':
      case 'workspace/didChangeConfiguration':
        break;
      default:
        break;
    }
  }
}

function clientCapabilities() {
  return {
    workspace: {
      applyEdit: false,
      // A plain boolean: this is the *client* capability shape. The
      // `{ supported, changeNotifications }` object form belongs to the
      // server's own `workspaceFolders` capability.
      workspaceFolders: true,
      configuration: true,
    },
    textDocument: {
      synchronization: {
        dynamicRegistration: false,
        willSave: false,
        willSaveWaitUntil: false,
        didSave: false,
      },
      publishDiagnostics: {
        dynamicRegistration: false,
        relatedInformation: true,
        versionSupport: false,
        tagSupport: { valueSet: [1, 2] },
        codeDescriptionSupport: true,
        dataSupport: true,
      },
      hover: {
        dynamicRegistration: false,
        contentFormat: ['markdown', 'plaintext'],
      },
      definition: { dynamicRegistration: false, linkSupport: false },
      references: { dynamicRegistration: false },
      documentSymbol: { dynamicRegistration: false, hierarchicalDocumentSymbolSupport: true },
      formatting: { dynamicRegistration: false },
      rangeFormatting: { dynamicRegistration: false },
      semanticTokens: {
        dynamicRegistration: false,
        requests: { range: false, full: true },
        tokenTypes: SEMANTIC_LEGEND.tokenTypes,
        tokenModifiers: SEMANTIC_LEGEND.tokenModifiers,
        formats: ['relative'],
        overlappingTokenSupport: false,
        multilineTokenSupport: false,
      },
    },
    window: {
      workDoneProgress: true,
      showMessage: { messageActionItem: { additionalPropertiesSupport: false } },
    },
  };
}

function rootUri() {
  const folders = vscode.workspace.workspaceFolders;
  return folders && folders.length ? folders[0].uri.toString() : null;
}

/**
 * Expand the variables a user naturally writes in `xulo.server.path`.
 * VS Code does not expand variables in a value an extension reads back, so
 * `${workspaceFolder}` and `~` are resolved here instead.
 */
function resolveServerPath(raw) {
  let value = String(raw == null ? '' : raw).trim();
  if (!value) return '';
  const folders = vscode.workspace.workspaceFolders;
  const root = folders && folders.length ? folders[0].uri.fsPath : '';
  if (root) value = value.replace(/\$\{workspaceFolder\}|\$\{workspaceRoot\}/g, root);
  if (value.startsWith('~/') || value === '~') {
    const os = require('os');
    value = os.homedir() + value.slice(1);
  }
  return value;
}

function workspaceFolders() {
  const folders = vscode.workspace.workspaceFolders;
  if (!folders || !folders.length) return null;
  return folders.map((f) => ({ uri: f.uri.toString(), name: f.name }));
}

module.exports = { activate, deactivate };
