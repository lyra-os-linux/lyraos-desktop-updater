# Lyra Upgrade

Workspace Rust do mecanismo de atualização recuperável do Lyra OS Desktop.
Os contratos normativos estão em
[`docs/lyra-upgrade-architecture.md`](../docs/lyra-upgrade-architecture.md) e
[`docs/lyra-upgrade-state-machine.md`](../docs/lyra-upgrade-state-machine.md).

- `core`: descoberta somente leitura, preflight, domínio, transições,
  planejamento determinístico e persistência atômica;
- `protocol`: requests e eventos versionados na fronteira do serviço;
- `cli`: cliente sem privilégios e ferramenta de diagnóstico;
- `service`: processo privilegiado autenticado por Polkit, vincula operações
  ao UID solicitante, revalida o plano e coordena zypper e Snapper;
- `offline`: aplica upgrades de versão previamente baixados e confirmados no
  `system-update.target`;
- `verifier`: valida RPM, dependências, boot e identidade da release antes de
  concluir uma operação após reinicialização;
- `src-tauri` e `ui`: interface sem privilégios, retomada de operação e console
  técnico sanitizado.

Execute os testes com:

```sh
cargo test --manifest-path upgrade/Cargo.toml --workspace
```

O pacote OBS de staging inclui unidades systemd para a transação offline e a
verificação pós-boot. A promoção para a imagem continua condicionada aos gates
de release e aos testes de falha em ambiente descartável.

`lyra-upgrade inspect` executa somente a allowlist de probes definida no core,
usa `LC_ALL=C`, não acessa a rede e não corrige o host. Metadados de
repositório permanecem não comprovados até a simulação do solver; portanto a
inspeção isolada falha de modo seguro em vez de liberar uma atualização.

O adaptador usa o XML estruturado do solver do zypper em modo dry-run. O
contrato bloqueia downgrade, remoção não autorizada, troca de vendor não
aprovada e quebra de pacotes lockstep. Antes da execução, o serviço atualiza os
metadados e exige que o hash do novo plano seja idêntico ao plano confirmado.
