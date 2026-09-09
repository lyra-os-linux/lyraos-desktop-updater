# Identidade após rollback

Uma recuperação mantém `source` e `target` como histórico da operação, mas
persiste `recovery` com o número e a identidade Btrfs do snapshot de origem
(ID, UUID e Parent UUID), e a identidade do clone gravável selecionado para
boot. A verificação de recuperação compara a release completa com `source`,
incluindo build, e exige que a raiz montada seja esse clone, que seu pai seja
o snapshot protegido e que o subvolume padrão ainda seja o selecionado.

O agendamento usa o mesmo lock das transações e relê o estado sob esse lock.
Antes de chamar Snapper, salva `rollback-preparing`, com `boot_snapshot=null`,
em `NeedsRecovery`. Depois de confirmar o resultado, salva
`rollback-scheduled`/`AwaitingReboot`. Falha intermediária conserva o objetivo
e a causa, sem repetir automaticamente o comando. Um objetivo incompleto
exige diagnóstico administrativo da seleção de boot.

Os registros de operação precisam estar em um subvolume Btrfs separado da
raiz, para sobreviver à restauração. O snapshot de origem precisa conter
`/usr/lib/lyra-upgrade/recovery-format` com versão `1`: esse marcador é
empacotado junto do verificador que entende o novo objetivo. Snapshots mais
antigos são recusados antes de mudar o boot; a recuperação administrativa
continua disponível. O leitor novo aceita operações antigas sem `recovery`,
mas recusa considerar um `rollback-scheduled` antigo como upgrade concluído.

Uma recuperação verificada termina em `Completed`, `Passed` e
`rollback-verified`. A resposta Status expõe `recovered=true` e a interface
mostra “Sistema restaurado”, com traduções inglês/português/espanhol.
A sequência de manifesto aplicada anteriormente permanece intacta: o upgrade
que falhou não passa a contar como aplicado. O caminho de upgrade normal
continua persistindo sua sequência após verificação bem-sucedida.

## Ensaio reproduzível

```sh
cargo build --locked -p lyra-upgrade-offline -p lyra-upgrade-verify \
  -p lyra-upgrade-service --bins --examples
python3 scripts/check-rollback-vm.py \
  --kernel /boot/vmlinuz-VERSAO --modules-dir /usr/lib/modules/VERSAO \
  --fixtures /caminho/fixtures/upgrade-vendor --binary-dir target/debug \
  --baseline-verifier /caminho/verificador-anterior \
  --log-dir /tmp/lyra-rollback-qualification
```

Usa os RPMs fictícios assinados da qualificação de caches offline. O primeiro
boot cria Btrfs com raiz e `/var` separados, protege A com Snapper, aplica o
RPM B usando o worker offline real e injeta falha na geração do GRUB.
O helper de teste chama a implementação de agendamento do serviço; recusa
executar fora da VM. O segundo cold boot monta o subvolume padrão selecionado
por Snapper e inicia systemd PID1 com a unit/verificador empacotados.

RPM, zypper, Btrfs, Snapper, systemd e verificador são reais. A identidade de
release B é uma fixture explícita: o RPM fictício altera seu próprio payload,
não o branding da distro. Secure Boot é um probe desabilitado, dracut é stub
e GRUB é a falha controlada; não há firmware/GRUB real nem ISO completa.
Nenhuma NIC, disco ou conta do host é incluída. O teste não autoriza executar
esses helpers, o verificador ou um rollback na estação de trabalho.

A auditoria #12 continua condição de publicação: o `zypper verify` existente
ainda pode reparar dependências. Este ensaio usa um conjunto consistente de
pacotes e não substitui a correção dessa semântica.

## Resultado da qualificação de 09/09/2026

Dois cold boots passaram com kernel `6.12.0-160100.4-default`, RPM e Snapper
da base Leap 16.1. O verificador anterior (`cf518f8`, auditoria #8) reproduziu
`POST_BOOT_IDENTITY_FAILED` no sistema A restaurado. O candidato concluiu em
`Completed`/`rollback-verified`, com RPM versão 1/vendor A e sequência aplicada
7 preservada, embora o manifesto do upgrade fosse 8.

Também passaram as recusas de snapshot sem marcador, estado dentro da raiz
que seria restaurada, intenção interrompida, build de origem divergente,
UUID de clone divergente, objetivo legado ausente e seleção padrão de boot
alterada. A unit terminou sem job pendente. O agendamento usa explicitamente
`--ambit classic --quiet --print-number`, evitando autodetecção ambígua de
raízes padrão ainda não listadas como snapshots gerenciados.
