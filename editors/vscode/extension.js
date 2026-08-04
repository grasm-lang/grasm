// Minimal VS Code client: spawns `grasm lsp` and speaks LSP over stdio.
const { workspace } = require('vscode');
const { LanguageClient, TransportKind } = require('vscode-languageclient/node');

let client;

function activate(context) {
  const command = workspace.getConfiguration('grasm').get('serverPath') || 'grasm';
  const serverOptions = {
    run: { command, args: ['lsp'], transport: TransportKind.stdio },
    debug: { command, args: ['lsp'], transport: TransportKind.stdio },
  };
  const clientOptions = {
    documentSelector: [{ scheme: 'file', language: 'grasm' }],
    synchronize: { fileEvents: workspace.createFileSystemWatcher('**/*.gr') },
  };
  client = new LanguageClient('grasm', 'Grasm Language Server', serverOptions, clientOptions);
  client.start();
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
