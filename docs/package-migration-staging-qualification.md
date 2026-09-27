# Updater 0.2.7 — qualificação em staging

A [PR26](https://github.com/lyra-os-linux/lyraos-desktop-updater/pull/26)
implementa a correção assinada de pacotes dentro da mesma versão do Lyra.
Em 27/09/2026, o RPM 0.2.7 publicado em staging passou pelos ensaios de
migração, recuperação por snapshot e coexistência com PackageKit. Esta etapa
qualifica o componente empacotado; a entrega do portal permanece pendente.

## Fontes e artefato

Fontes geradas por `packaging/make-obs-sources.sh` a partir do merge limpo
`9ddc4000c5b8a051dba1bfa365b1c5db824cac0d`, publicadas exclusivamente no
[OBS staging, revisão 40](https://build.opensuse.org/package/show/home:rodrigosbrito:lyra:staging/lyra-upgrade?rev=40),
srcmd5 `69dca165aed45064db106ae96dc015fc`. Todos os arquivos enviados foram
baixados novamente e comparados por SHA256. As 433 dependências externas Cargo
e o conteúdo, permissões e links das 28.146 entradas vendor são idênticos aos
da 0.2.6. A compactação muda pelo timestamp normalizado do commit.
A chave pública de manifests permanece inalterada.

Artefato: `lyra-upgrade-0.2.7-lp161.1.1.x86_64.rpm`, SHA256
`ce703c2adffd04c2a98d2430bc22663a1b3d838424f1820e465e443ed15114d6`.
As assinaturas dos metadados e do RPM foram verificadas com a chave OBS
`399218A6E088C4053F4533BE58097F767EDCA82E`. O download público corresponde
byte a byte ao da API de build. DISTURL/srcmd5, `build-source.txt`, units,
chave de manifests e registro systemd pré-instalação correspondem às fontes.

O build offline completo, incluindo a interface, passou com 145 testes Rust.
Os apontamentos rpmlint são os mesmos da 0.2.6: seis avisos de binários com
símbolos e um erro `polkit-untracked-privilege` para a ação própria do Lyra.
Nenhum filtro foi acrescentado e a política de autorização não mudou.
Os gates de staging passaram para Lyra (23 pacotes), Vega (6) e Fina (1),
todos publicados. Identidades, hashes e resultados estão na
[evidência estruturada](package-migration-staging-evidence.json).

## Ensaios com os binários do RPM

Serviço, worker offline e verificador foram extraídos do RPM assinado para
um diretório privado. Seus hashes correspondem aos registrados pelos ensaios;
não foram substituídos pelos binários compilados localmente.
Kernel dos guests: `6.12.0-160100.5-default`.

| Ensaio | Boots | Resultado |
| --- | ---: | --- |
| Migração dentro da 1.1 | 3 | Plano confirmado, snapshot, aplicação offline e inventário final corretos; sequência 7→8 somente após boot verificado |
| Recuperação explícita | 4 | Falha de inventário detectada no boot; rollback Snapper restaura clone e inventário original; sequência permanece 7 |
| PackageKit offline | 3 | Transação nativa e dois reinícios; marcador removido pelo PackageKit e coexistência correta da unit Lyra |

Os dois ensaios de migração usam RPMs inertes assinados, oferta HTTPS e chave
de manifesto efêmera dentro do guest. O serviço recusa execução sem confirmação
e recalcula o plano. Alterar o documento assinado ou o payload em cache bloqueia
a aplicação sem mudar o pacote instalado. Após restaurar a fixture descartável,
a transação autorizada passa. A recuperação injeta uma versão inesperada após
a aplicação e verifica `POST_BOOT_INVENTORY_FAILED`, seguido de rollback
explícito pelo serviço e validação do snapshot no quarto boot.

O ensaio PackageKit usa PackageKit/zypp/systemd nativos e a unit Lyra distribuída.
Também passam os casos negativos de propriedade do marcador e estado inválido.
Todos os discos são Btrfs descartáveis, com serial e marcador de kernel
verificados antes da formatação. Os harnesses removem discos e chaves efêmeras
ao terminar. Nenhum RPM foi instalado na estação.

## Reprodução e limites

Verificar primeiro a assinatura dos metadados e do RPM com a chave fixada,
SHA256, identidade, DISTURL e proveniência. Extrair somente em diretório privado.
Gerar os RPMs inertes conforme a [qualificação do componente](evidence/package-migration/README.md),
então executar, em diretórios de saída novos:

```sh
python3 scripts/check-package-migration-vm.py \
  --kernel /boot/vmlinuz-6.12.0-160100.5-default \
  --modules-dir /usr/lib/modules/6.12.0-160100.5-default \
  --fixtures /caminho/migration-native/fixtures/upgrade-vendor \
  --binary-dir /caminho/rpm-root/usr/libexec \
  --log-dir /caminho/evidencia/migration-success --scenario success --accel kvm
python3 scripts/check-package-migration-vm.py \
  --kernel /boot/vmlinuz-6.12.0-160100.5-default \
  --modules-dir /usr/lib/modules/6.12.0-160100.5-default \
  --fixtures /caminho/migration-native/fixtures/upgrade-vendor \
  --binary-dir /caminho/rpm-root/usr/libexec \
  --log-dir /caminho/evidencia/migration-rollback --scenario rollback --accel kvm
python3 scripts/check-packagekit-offline-vm.py \
  --kernel /boot/vmlinuz-6.12.0-160100.5-default \
  --modules-dir /usr/lib/modules/6.12.0-160100.5-default \
  --offline-binary /caminho/rpm-root/usr/libexec/lyra-upgrade-offline \
  --output-dir /caminho/evidencia/packagekit-vm
```

A migração usa UID autenticado de fixture, probe Secure Boot desabilitado e
arquivo GRUB de fixture. O harness invoca o worker no ambiente offline do guest;
não qualifica a interação Polkit, regeneração do bootloader ou a orquestração
completa da sessão de um desktop instalado. Os RPMs são inertes e a chave de
manifesto é exclusiva do teste. O ensaio PackageKit qualifica sua própria
transação offline, sem substituir o ensaio de entrega real do portal.

## Próximos gates

Preparar e revisar o manifesto real do portal com mínimo Updater 0.2.7,
identidade Lyra 1.1 preservada, RPMs/hashes exatos e traduções somente quando
instaladas. Assinar pelo fluxo existente da chave oficial e executar
autorização, staging, offline, boot e recuperação em baseline Btrfs completo,
com e sem traduções. Depois vêm a promoção revisada e os ensaios da candidata
ISO de checksum exato. Desktop #125, #6/#102 e Updater #22 continuam abertos.

Release permanece na 0.2.5, revisão 12, srcmd5
`7858b2697376f5c70a42e5796ee2e343`. Esta qualificação não publica manifesto,
não eleva o mínimo da imagem e não habilita proteção parental. Em caso de
regressão, bloquear a promoção, corrigir/reconstruir em staging e repetir a
qualificação do novo artefato.
