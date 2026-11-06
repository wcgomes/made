# grok-made

Plugin mínimo do Grok para a prova de conceito do Made. Cada evento de sessão
acrescenta uma linha em JSON com `sessionId`, `cwd` e `status`
(`idle`, `working`, `attention`, `error`).

O hook é um executável nativo, `hooks/bin/made-grok-status`
(`made-grok-status.exe` no Windows), compilado para o sistema e a arquitetura
de quem instala o plugin. Ele não precisa de Python nem de nenhum outro
interpretador.

A pasta `plugins/grok-made` do repositório guarda só o código-fonte do plugin
e não contém o executável. O Made instala sozinho a cópia pronta, e quem
instala à mão usa a cópia montada, que fica em `<install_root>/plugins/grok-made`
(em `target/<perfil>/plugins/grok-made` ao compilar com o cargo). Para montar
essa cópia depois de `cargo build -p made-grok-hook`:

```sh
.github/scripts/stage-grok-plugin.sh x86_64-unknown-linux-gnu \
  target/debug/made-grok-status target/debug/plugins/grok-made
```

Instale a cópia montada e reinicie o Grok:

```sh
grok plugin install /caminho/para/target/debug/plugins/grok-made --trust
```

Para remover: `grok plugin uninstall grok-made --confirm`.

O log fica em `~/.grok/made-grok-status.jsonl` (ou em
`$GROK_HOME/made-grok-status.jsonl`). O Made lê esse arquivo.

```sh
tail -f ~/.grok/made-grok-status.jsonl
```

Abra duas ou três sessões no dashboard e mande um prompt em cada uma. Cada
linha deve trazer o id da sessão que gerou o evento.
