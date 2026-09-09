# Qualificação de cache e repositórios offline

O ensaio usa o executável real `lyra-upgrade-offline`, RPMs de teste assinados,
RPM/zypper/Snapper nativos e um disco Btrfs descartável. QEMU inicia sem placa
de rede (`-nic none`), sem montar diretórios ou discos do host no guest.
O worker nunca deve ser executado diretamente no host para este teste.

Os cenários verificam:

- aplicação sem cache global e sem rede;
- cache global inválido ignorado, sem alteração dos seus arquivos;
- metadados preparados corrompidos ou ausentes;
- falha de preflight, repetição recusada e nova tentativa explicitamente preparada;
- RPM ausente sem publicação antecipada dos repositórios de destino;
- mudança real nos bloqueios rejeitada pelo hash do plano;
- reprodução opcional da falha com o executável anterior à correção.

Sucesso exige versão/vendor novos no banco RPM, `AwaitingReboot`, remoção do
marcador e backup exato dos repositórios anteriores. Falha antes de aplicar
exige vendor anterior, repositórios intactos, `NeedsRecovery`, retirada do
marcador e nenhuma reconstrução de boot. O helper de planejamento também
confere que aquecer o cache não modifica o inventário.

## Execução em Leap 16.1

São necessários Rust, Python 3, GnuPG/rpmsign/rpmbuild/rpm, zypper,
QEMU x86_64, cpio, util-linux, kmod, Btrfs, Snapper, gzip, xz, zstd e um kernel
com seus módulos correspondentes. O harness copia as ferramentas locais e
suas bibliotecas; não é uma imagem portátil para hosts de outras distros.

Use um diretório de fixtures novo e execute como usuário normal:

```sh
unshare --user --map-root-user python3 scripts/check-vendor-policy-native.py \
  --export-vm --output /tmp/lyra-offline-fixtures
cargo build --locked -p lyra-upgrade-offline -p lyra-upgrade-service \
  --bins --example offline-vm-plan
python3 scripts/check-offline-cache-vm.py \
  --kernel /usr/lib/modules/VERSAO/vmlinuz \
  --modules-dir /usr/lib/modules/VERSAO \
  --fixtures /tmp/lyra-offline-fixtures/upgrade-vendor \
  --offline-binary target/debug/lyra-upgrade-offline \
  --planner-binary target/debug/examples/offline-vm-plan \
  --log /tmp/lyra-offline-vm.log
```

Opcionalmente, `--baseline-binary /caminho/worker-anterior` inclui a reprodução
da regressão. O harness encerra com falha se algum cenário falhar ou o guest
não emitir o marcador final de sucesso. A CI comum executa os testes Rust e
contratos de empacotamento; este ensaio nativo requer o ambiente acima.

## Limites

O probe de Secure Boot informa desabilitado e os comandos dracut/GRUB são
substituídos por registros de chamada. O ensaio valida a ordem de execução,
mas não qualifica esses componentes nem uma atualização completa da ISO.
A retomada é preparada explicitamente no guest; não representa uma nova
interface pública de retry. Os RPMs de fixture não possuem scripts ou
dependências. Falhas parciais de scripts RPM, queda de energia, rollback e a
verificação do segundo boot continuam exigindo os respectivos gates.

## Resultado de 09/09/2026

Oito cenários passaram no kernel `6.12.0-160100.4-default`, em 56,127 segundos
de testes dentro do guest. O executável anterior, de
`ea497d1a68fbd9d23bea23500c0af1b438b5418e`, reproduziu
`RepositoryMetadataInvalid { alias: "old-source" }` usando os mesmos dados.
O candidato aplicou o RPM 2-1 do fornecedor B; as falhas anteriores à
aplicação conservaram o RPM 1-1 do fornecedor A e os repositórios originais.
O guest encerrou com `LYRA_OFFLINE_CACHE_VM_EXIT=0`.
