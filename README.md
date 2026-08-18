# Lyra Upgrade

Workspace Rust do mecanismo de atualização recuperável do Lyra OS Desktop.
Os contratos normativos estão em
[`docs/lyra-upgrade-architecture.md`](../docs/lyra-upgrade-architecture.md) e
[`docs/lyra-upgrade-state-machine.md`](../docs/lyra-upgrade-state-machine.md).

- `core`: descoberta somente leitura, preflight, domínio, transições,
  planejamento determinístico e persistência atômica;
- `protocol`: requests e eventos versionados na fronteira do serviço;
- `cli`: cliente sem privilégios e ferramenta de diagnóstico;
- `service`: processo privilegiado; nesta etapa aceita apenas inspeção e não
  executa comandos do host.

Execute os testes com:

```sh
cargo test --manifest-path upgrade/Cargo.toml --workspace
```

O workspace ainda não é incluído na ISO. Empacotamento e ativação só serão
feitos quando preflight, persistência e testes de falha estiverem completos.

`lyra-upgrade inspect` executa somente a allowlist de probes definida no core,
usa `LC_ALL=C`, não acessa a rede e não corrige o host. Metadados de
repositório permanecem não comprovados até a simulação do solver; portanto a
inspeção isolada falha de modo seguro em vez de liberar uma atualização.

O contrato do solver já bloqueia downgrade, remoção não autorizada, troca de
vendor não aprovada e quebra de pacotes lockstep. A adaptação concreta para
libzypp/libsolv ainda não está ligada; até isso ocorrer, nenhum plano pode ser
promovido para execução.
