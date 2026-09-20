# Coexistência offline com PackageKit — Updater #22

A versão 0.2.5 identifica o proprietário de `/system-update` antes de adquirir
o bloqueio Lyra ou abrir seu diretório de operações. Marcador ausente ou
externo encerra com sucesso, mesmo com destino inexistente, estado Lyra
corrompido ou diretório de operações ausente. Nenhum dado externo é alterado.
Links relativos são interpretados a partir do diretório do marcador.

Pedidos no namespace Lyra continuam exigindo uma operação válida diretamente
sob o diretório de estado. Diretórios ausentes, caminhos aninhados, destinos
que escapam desse diretório e estado corrompido são erros. A propriedade é
verificada novamente após o bloqueio, antes da recuperação e antes de remover
o marcador; uma substituição por outro pedido não autoriza removê-lo ou marcar
outra operação como falha. Isso é uma revalidação, não uma promessa de exclusão
atômica com outros processos administrativos que alterem o link simultaneamente.

A unit continua habilitada. Não houve alteração no PackageKit nem no protocolo
de confirmação, assinatura de manifests, transação ou recuperação do Lyra.
O comportamento segue o
[contrato de atualizações offline do systemd](https://raw.githubusercontent.com/systemd/systemd/main/man/systemd.offline-updates.xml).

## Evidência de 19/09/2026

Uma VM mínima com executáveis nativos PackageKit 1.2.8/zypp, RPM e systemd do
Leap 16.1 executou três boots sobre o mesmo disco Btrfs descartável:

1. instalar a versão 1 de um RPM inerte local, baixar a versão 2 com
   `pkcon --only-download update` e preparar o pedido por `pkcon offline-trigger`;
2. o gerador nativo do systemd selecionar `system-update.target`, iniciar as
   units Lyra/PackageKit, aplicar a atualização e reiniciar;
3. verificar `Success=true`, a versão 2 no banco RPM, remoção do marcador pelo
   PackageKit e resultado persistido da unit Lyra.

Com o binário publicado 0.2.4-lp161.1.1, Lyra falha com ENOENT e exit 1;
PackageKit conclui. Com o novo binário, Lyra retorna 0 e PackageKit conclui.
Ambos os ensaios confirmam os dois reinícios. O ensaio corrigido cobre também
marcador ausente, removido ou externo com bloqueio Lyra ocupado; estado Lyra
corrompido ignorado para pedido externo e rejeitado para pedido próprio;
e um estado ReadyToReboot reconhecido que entra em NeedsRecovery quando seu
manifest obrigatório falta. Nenhum pacote é alterado por esses casos negativos.

127 testes Rust, 30 Python, rustfmt e Clippy passaram. Os testes unitários
cobrem a resolução, links relativos, caminhos inválidos e preservação de
marcadores substituídos. Hashes dos logs e limites estão em
[packagekit-offline-evidence.json](packagekit-offline-evidence.json).

## Reprodução

Com kernel e módulos correspondentes, QEMU/KVM, rpmbuild e ferramentas nativas
PackageKit/zypp instaladas no host:

```sh
cargo build --locked -p lyra-upgrade-offline
python3 scripts/check-packagekit-offline-vm.py \
  --kernel /boot/vmlinuz-6.12.0-160100.4-default \
  --modules-dir /usr/lib/modules/6.12.0-160100.4-default \
  --offline-binary target/debug/lyra-upgrade-offline \
  --output-dir /tmp/lyra-packagekit-evidence
```

Para reproduzir o defeito, fornecer o binário anterior por `--offline-binary`
e acrescentar `--expect-lyra-failure`. Ele é copiado e executado somente na VM.
A imagem não copia contas, repositórios, banco RPM ou dados pessoais do host;
não tem interface de rede. O disco tem serial de teste conferido antes de
formatar. O único RPM atualizado contém um arquivo de texto; gpgcheck fica
desabilitado apenas nesse repositório fictício isolado. A política de assinatura
do produto permanece intacta. Discos e initramfs são removidos ao terminar.
A coleta acrescenta apenas um ExecStopPost e redirecionamento de log à unit
Lyra; as ações PackageKit/zypp e os reboots são reais.

## Publicação de 20/09/2026

Publicado `lyra-upgrade-0.2.5-lp161.1.1.x86_64.rpm` pelo
[pedido OBS1379300](https://build.opensuse.org/request/show/1379300).
Fontes `8cb542fc5f24916da441e4027c0218d48cbc276e`, staging rev38 e release rev12,
ambos com srcmd5 `7858b2697376f5c70a42e5796ee2e343`.

O worker extraído do RPM assinado de staging passou pelo mesmo ciclo nativo
PackageKit com dois reboots e cenários negativos; o worker de release é
idêntico por SHA256. Os 127 testes Rust passaram nos dois builds. A assinatura,
proveniência, units, chave pública e registro systemd pré-instalação foram
verificados. O download público é idêntico ao RPM da API, SHA256
`750ef116be969dc41521404049fef5fec443ae6458e8412e8883470dde3d5b55`.
Ver [evidência OBS](packagekit-obs-evidence.json).

A revisão inicial37 foi substituída para corrigir a ausência de `%systemd_pre`.
Restam apontamentos rpmlint já presentes na0.2.4: ação própria não listada no
perfil Polkit upstream e seis binários com símbolos. A política de autorização
não foi alterada e nenhum apontamento foi ocultado por filtro novo.

## Integração pendente

O [Desktop PR94](https://github.com/lyra-os-linux/lyraos-desktop/pull/94)
exige Updater>=0.2.5 e registra UPD-01. Este ensaio de componentes não equivale
à migração completa para um sucessor Lyra assinado, desktop GNOME ou ISO final.
Integrar os PRs e repetir o ciclo sobre o checksum exato da candidata antes de
encerrar #22. Não houve instalação na estação. Em regressão, reverter fontes,
reconstruir pelo staging e repetir os gates; manter a candidata bloqueada.
