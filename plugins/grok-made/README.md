# grok-made

Plugin mínimo do Grok para a prova de conceito do Made. Cada evento de sessão
acrescenta uma linha em JSON com `sessionId`, `cwd` e `status`
(`idle`, `working`, `attention`, `error`).

Instale a pasta do repositório e reinicie o Grok:

```sh
grok plugin install /Users/wyller/Repos/made/plugins/grok-made --trust
```

O log fica em `~/.grok/made-grok-status.jsonl`. O Made lê esse arquivo.

```sh
tail -f ~/.grok/made-grok-status.jsonl
```

Abra duas ou três sessões no dashboard e mande um prompt em cada uma. Cada
linha deve trazer o id da sessão que gerou o evento.
