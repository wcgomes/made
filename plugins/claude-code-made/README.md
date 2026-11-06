# claude-code-made

Plugin do Claude Code que o Made carrega só para a sessão. Cada evento de
sessão acrescenta uma linha em JSON ao spool da aba do Made que iniciou o
Claude Code, com apenas o horário, `sessionId`, `cwd`, o evento, o tipo de
notificação, o `status` (`idle`, `working`, `attention`, `error`, `ended`), o
título da sessão e a cadeia de pids. O hook só registra metadados: não grava
prompt, entrada ou saída de ferramentas, texto de notificação nem transcrição.

O Made carrega o plugin por `CLAUDE_CODE_PLUGIN_DIRS`, definida no ambiente das
abas locais, e aponta o spool em `MADE_CLAUDE_SPOOL`. Nada é instalado na
configuração do Claude Code: não há `claude plugin install` e não há nada para
desinstalar. O Claude Code iniciado fora do Made não carrega este plugin, e sem
`MADE_CLAUDE_SPOOL` o hook não grava nada.

O hook é um executável nativo, `bin/made-claude-status`
(`made-claude-status.exe` no Windows), assíncrono, que sempre sai com 0 e não
escreve na saída padrão. Ele não precisa de nenhum interpretador.

A pasta `plugins/claude-code-made` do repositório guarda só o código-fonte do
plugin e não contém o executável. O Made usa a cópia montada em
`<install_root>/plugins/claude-code-made`. Para montá-la depois de
`cargo build -p made-claude-hook`:

```sh
.github/scripts/stage-claude-plugin.sh x86_64-unknown-linux-gnu \
  target/debug/made-claude-status target/debug/plugins/claude-code-made
```
