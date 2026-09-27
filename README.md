# Lyra Upgrade

Workspace Rust do mecanismo de atualização recuperável do Lyra OS Desktop.
Os contratos normativos estão em
[`docs/lyra-upgrade-architecture.md`](docs/lyra-upgrade-architecture.md) e
[`docs/lyra-upgrade-state-machine.md`](docs/lyra-upgrade-state-machine.md).

- `core`: descoberta somente leitura, preflight, domínio, transições,
  planejamento determinístico e persistência atômica;
- `protocol`: requests e eventos versionados na fronteira do serviço;
- `cli`: cliente sem privilégios e ferramenta de diagnóstico;
- `service`: consultas/planejamento sem root e executor autenticado por Polkit, vincula operações
  ao UID solicitante, revalida o plano e coordena zypper e Snapper;
- `offline`: aplica upgrades de versão previamente baixados e confirmados no
  `system-update.target`;
- `verifier`: valida RPM, dependências, boot e identidade da release antes de
  concluir uma operação após reinicialização;
- `src-tauri` e `ui`: interface sem privilégios, retomada de operação e console
  técnico sanitizado.

Execute os testes com:

```sh
cargo test --workspace
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

Os fornecedores são identificados pelo cabeçalho RPM instalado e pelo registro
RPM-MD da versão, arquitetura e repositório propostos. Identidades ausentes ou
ambíguas bloqueiam a operação. Consulte
[as fixtures e a qualificação nativa](service/tests/fixtures/vendor-policy/README.md).

A [qualificação offline em VM](docs/offline-cache-qualification.md) executa o
worker real sem rede, verifica o cache preparado e a preservação dos
repositórios em falhas anteriores à aplicação.

A [qualificação do verificador de boot](docs/boot-verifier-qualification.md)
reproduz a espera circular anterior e exercita conclusão, degradação e timeout
com systemd real em uma VM descartável.

A implementação 0.2.4 do [aceite Desktop #13](docs/desktop-13-acceptance.md) usa
protocolo/plano v3. Abrir, consultar, planejar e acompanhar não solicitam senha.
Autenticação ocorre somente nas ações administrativas explícitas. A promoção
aguarda os ensaios GNOME de sucessor/boot/rollback nas issues Desktop #24/#26.

A [qualificação de coexistência com PackageKit](docs/packagekit-offline-qualification.md)
reproduz a falha anterior e valida a saída sem efeitos para pedidos de outra
ferramenta durante um ciclo offline com reinicializações reais.

A versão 0.2.6 acrescenta autorização de troca de fornecedor por nomes exatos
no manifesto assinado. A regra pode liberar somente o portal e suas traduções,
sem permitir a mesma troca para outros RPMs. Manifestos anteriores continuam
compatíveis; listas inválidas ou regras amplas que anulem a restrição bloqueiam.
Veja o [contrato do solver](docs/lyra-upgrade-architecture.md#contrato-do-solver).

A versão 0.2.7 acrescenta [migrações de pacotes dentro da mesma versão do Lyra](docs/package-migration.md),
com plano completo e payloads limitados às identidades assinadas.
O [RPM 0.2.7 foi qualificado em staging](docs/package-migration-staging-qualification.md):
145 testes Rust no OBS, migração e recuperação Btrfs/Snapper com seus binários,
além da regressão PackageKit. O manifesto real do portal, sua migração com e
sem traduções e a candidata ISO ainda precisam passar pelos respectivos gates.
O canal release continua na 0.2.5.

A versão 0.2.8 corrige a [consulta ao Snapper durante o planejamento sem
privilégios](docs/snapper-readiness-qualification.md), mantendo a recusa quando
não for possível verificar a configuração de recuperação.
