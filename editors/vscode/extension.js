// Aura VS Code extension — spawns `aura lsp` over stdio.
// Zero build step: plain CommonJS, activated on `onLanguage:aura`.

const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

/** @type {LanguageClient | undefined} */
let client;

/** @param {vscode.ExtensionContext} context */
function activate(context) {
  const config = vscode.workspace.getConfiguration("aura");
  const serverPath = config.get("serverPath", "aura");

  /** @type {import("vscode-languageclient/node").ServerOptions} */
  const serverOptions = {
    command: serverPath,
    args: ["lsp"],
    transport: TransportKind.stdio,
  };

  /** @type {import("vscode-languageclient/node").LanguageClientOptions} */
  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "aura" }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.aura"),
    },
    outputChannelName: "Aura",
  };

  client = new LanguageClient(
    "aura-lsp",
    "Aura Language Server",
    serverOptions,
    clientOptions,
  );

  context.subscriptions.push(client.start());
}

function deactivate() {
  if (client) {
    return client.stop();
  }
  return undefined;
}

module.exports = { activate, deactivate };
