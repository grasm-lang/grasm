// Minimal VS Code client: spawns `grasm lsp` and speaks LSP over stdio, and
// `grasm dap` for the debugger.
const { workspace, debug, DebugAdapterExecutable, DebugAdapterDescriptorFactory } = require('vscode');
const { LanguageClient, TransportKind } = require('vscode-languageclient/node');

let client;

function grasmPath() {
  return workspace.getConfiguration('grasm').get('serverPath') || 'grasm';
}

function activate(context) {
  const command = grasmPath();
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

  // The same binary, speaking the debug protocol on stdio. Resolved at launch
  // rather than at activation so a changed `grasm.serverPath` takes effect
  // without reloading the window.
  context.subscriptions.push(
    debug.registerDebugAdapterDescriptorFactory('grasm', {
      createDebugAdapterDescriptor() {
        return new DebugAdapterExecutable(grasmPath(), ['dap']);
      },
    })
  );

  // `F5` on a .gr file with no launch.json runs that file.
  context.subscriptions.push(
    debug.registerDebugConfigurationProvider('grasm', {
      resolveDebugConfiguration(_folder, config) {
        if (!config.type && !config.request && !config.name) {
          const editor = require('vscode').window.activeTextEditor;
          if (editor && editor.document.languageId === 'grasm') {
            config.type = 'grasm';
            config.name = 'Run this file';
            config.request = 'launch';
            config.program = editor.document.fileName;
          }
        }
        return config;
      },
    })
  );
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
