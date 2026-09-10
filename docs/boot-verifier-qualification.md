# Verificador pós-boot: espera limitada

A unit executa depois de `multi-user.target`. Consulta se esse alvo está ativo
e enumera unidades já falhadas, sem usar `is-system-running --wait`. A fila
global de inicialização pode conter o próprio job do verificador; aguardar
`StartupFinished` nessa posição impede que o job termine.

O processo principal persiste `VerifyingBoot` e supervisiona um worker com
GNU `timeout`, já fornecido por `coreutils`, dependência do RPM. O prazo de
180 segundos cobre todos os probes, inclusive a descoberta. Ao expirar,
envia TERM ao grupo, com KILL após cinco segundos se o worker não encerrar.
O pai persiste `NeedsRecovery`,
`boot_verification=Failed` e `POST_BOOT_VERIFICATION_TIMEOUT`. A unit possui
um limite externo de 200 segundos, `KillMode=control-group` e dez segundos
para encerrar descendentes que tenham sobrevivido à saída do worker.

Falhas de descoberta, identidade, filesystem, RPMDB, dependências, alvo de
boot, unidades falhadas e GRUB recebem códigos específicos no estado e no
journal. Falha de verificação também encerra a unit com código não zero.
Um upgrade bem-sucedido avança para `Completed` e persiste a sequência do
manifesto. Uma recuperação verificada usa a identidade de origem e mantém a
sequência anterior; ver [qualificação de rollback](rollback-qualification.md).

## Ensaio em VM

`scripts/check-boot-verifier-vm.py` inicia um novo kernel com systemd real
como PID1, o executável real do verificador e a unit empacotada. Não há NIC,
disco do host, conta do host ou repositório remoto. A operação pendente já
existe ao iniciar o systemd. O observador é `Type=simple`, portanto não mantém
um job de inicialização pendente enquanto observa o verificador.

Os probes de RPM/zypper, filesystem, Snapper, Secure Boot e o arquivo GRUB
são fixtures controladas. Este teste qualifica a integração com systemd,
supervisão, sinais e avanço do estado em arquivos do guest; não qualifica
RPMs, firmware, snapshots ou durabilidade após perda de energia.

```sh
cargo build --locked -p lyra-upgrade-verify
python3 scripts/check-boot-verifier-vm.py \
  --kernel /usr/lib/modules/VERSAO/vmlinuz \
  --binary target/debug/lyra-upgrade-verify \
  --scenario healthy --log /tmp/lyra-verify-healthy.log
```

Repetir com `--scenario degraded` e `--scenario timeout`. Este último usa o
prazo real de 180 segundos e um probe que ignora TERM, exigindo KILL. Para
reproduzir a regressão, usar `--scenario baseline`, o executável anterior e
`--baseline-unit /caminho/unit-anterior`. O baseline deve permanecer
`activating`, com job pendente, sistema `starting` e estado `AwaitingReboot`.

O candidato saudável deve concluir; o degradado deve persistir
`POST_BOOT_FAILED_UNITS`; o travado deve persistir timeout e não deixar
processos do probe vivos. As units devem terminar, removendo seus jobs.

## Gates separados

A consulta de saúde é pontual: serviços iniciados posteriormente podem falhar
depois dela. Cold boot completo da ISO e rollback continuam sendo gates
independentes. O comportamento existente de `zypper verify` ainda exige a
correção e qualificação da auditoria #12, para impedir reparos e interpretar
dependências quebradas sem escrita. A conclusão de #8 não libera publicação
do atualizador nem substitui esse requisito.
