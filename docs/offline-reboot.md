# Reinício após a aplicação offline

No ensaio do RPM 0.2.8 com o manifesto oficial do portal, a unidade aplicou o
pacote e persistiu `AwaitingReboot`, mas não solicitou o reinício. O marcador
`/system-update` já havia sido removido, portanto `system-update-cleanup.service`
ignorou sua condição. A máquina permaneceu em `system-update.target`, sem jobs
pendentes. Um reinício manual permitiu ao verificador concluir a operação.

A versão 0.2.9 passa `--reboot` ao worker na unidade systemd. Após a aplicação,
o worker solicita `/usr/bin/systemctl --no-block reboot`. No caminho de erro,
primeiro persiste `NeedsRecovery` quando o estado é legível e remove somente o
marcador da própria operação. Falhas ao persistir essa recuperação ou remover
o marcador impedem a solicitação de reinício. Estado originalmente ilegível
continua preservado para diagnóstico, com remoção do marcador próprio, como
nas versões anteriores.

A resolução de propriedade precede o lock e qualquer alteração de estado.
Marcadores ausentes ou pertencentes a outro atualizador retornam sucesso sem
solicitar reinício, inclusive com `--reboot`. Uma nova verificação antes do
comando impede reinício sobre marcador substituído, incluindo symlink pendente.
O comando sem argumentos mantém a execução manual sem reinício automático.
Falha na solicitação de reboot não reclassifica uma aplicação concluída como
falha de aplicação; seu estado `AwaitingReboot` permanece disponível.

## Validação

- 151 testes Rust, 35 testes Python/UI, rustfmt e Clippy aprovados.
- Testes de encerramento cobrem sucesso, falha de aplicação, falha ao registrar
  recuperação, falha ao solicitar reboot e marcador substituído.
- VM nativa PackageKit/zypp: preparação, aplicação offline e boot de verificação
  aprovados com a unidade corrigida. Pedidos estrangeiros/ausentes com `--reboot`
  permanecem sem efeito mesmo com lock ocupado ou estado Lyra corrompido.
- Ensaio de operação própria com manifesto oficial em VM UEFI/Btrfs em andamento.

Esta correção ainda não qualifica um RPM 0.2.9 publicado, recuperação via GRUB,
ISO ou promoção do canal release. A fixture usa SELinux permissivo e autenticação
Polkit por agente TTY nativo separado; não cobre o diálogo gráfico do GNOME.
