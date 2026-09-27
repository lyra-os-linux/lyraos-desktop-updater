# Updater 0.2.6 — qualificação em staging

A [PR24](https://github.com/lyra-os-linux/lyraos-desktop-updater/pull/24)
acrescenta nomes exatos de pacotes às autorizações de troca de fornecedor do
manifesto assinado. Isso permite preparar a exceção do portal GNOME sem
autorizar a mesma troca para outros pacotes. O manifesto que usa `packages`
exige Updater >=0.2.6; regras legadas sem esse campo continuam compatíveis.

## Fontes

Fontes geradas por `packaging/make-obs-sources.sh` a partir do merge limpo
`5334acdb387549ca2ba56a6c61a5e85a4f7f8e6e`, incluindo a correção PackageKit da
0.2.5. Destino exclusivo:
[OBS staging, revisão 39](https://build.opensuse.org/package/show/home:rodrigosbrito:lyra:staging/lyra-upgrade?rev=39),
srcmd5 `8263d6f2a9e64f2b2af9ee8f9a196730`.

Todos os arquivos enviados foram baixados novamente e comparados por SHA256.
As dependências externas do Cargo.lock e os conteúdos, permissões e links
das 28.146 entradas do arquivo vendor são idênticos aos da 0.2.5. O arquivo
compactado difere porque usa o timestamp normalizado do novo commit.
A chave pública de manifests permanece inalterada.

## Artefato de 27/09/2026

`lyra-upgrade-0.2.6-lp161.1.1.x86_64.rpm`, SHA256
`30740d14ec768d54e5444034411d89baa6480b0ec7804b265c1d42fd3da2d65e`.
Assinaturas dos metadados e do pacote verificadas com a chave fixada abaixo;
o RPM público é idêntico ao da API. Proveniência, units, chave de manifests e
registro systemd pré-instalação correspondem às fontes aprovadas.

O build completo, incluindo a interface, passou com 133 testes Rust. Os gates
de staging passaram para Lyra (23 pacotes), Vega (6) e Fina (1), todos publicados.
Continuam os apontamentos rpmlint anteriores: uma ação Polkit própria ausente
da lista upstream e seis binários com símbolos. Nenhum filtro foi acrescentado
e a política de autorização não mudou.

O worker extraído do RPM passou pelo ciclo nativo PackageKit com dois reinícios
no kernel `6.12.0-160100.5-default`. A transação atualizou o RPM inerte 1→2,
o PackageKit removeu seu marcador e a unit Lyra terminou com sucesso.
Os cenários negativos de marcador/estado também passaram. O disco de teste
foi removido pelo harness. Hashes, identidade e limites estão na
[evidência estruturada](vendor-scope-staging-evidence.json).

## Reprodução e alcance

O artefato deve ser verificado com a chave OBS de fingerprint
`399218A6E088C4053F4533BE58097F767EDCA82E`, em bancos GPG/RPM temporários.
Conferir assinatura dos metadados do repositório e do RPM, identidade,
DISTURL/srcmd5, `build-source.txt`, chave de manifests, units e scriptlet de
registro systemd. O download público de staging deve ser idêntico ao da API
de build. Extrair o RPM apenas em um diretório privado.

Com o worker extraído, executar o harness existente em uma VM descartável:

```sh
python3 scripts/check-packagekit-offline-vm.py \
  --kernel /boot/vmlinuz-6.12.0-160100.5-default \
  --modules-dir /usr/lib/modules/6.12.0-160100.5-default \
  --offline-binary /caminho/rpm-root/usr/libexec/lyra-upgrade-offline \
  --output-dir /caminho/evidencia/packagekit-vm
```

O harness usa PackageKit/zypp/systemd nativos e três boots sobre um disco
Btrfs descartável, sem rede. Apenas um RPM inerte do teste é atualizado.
Os cenários negativos verificam a propriedade do marcador offline e a
recuperação de estado Lyra inválido. O processo da estação não executa o
worker nem instala o pacote.

## Gates que continuam pendentes

Esta etapa qualifica o empacotamento da capacidade e a regressão PackageKit.
Os testes Rust cobrem assinatura, plano confirmado, nomes exatos, regras
inválidas e revalidação da política. O ensaio PackageKit não exerce a migração
assinada SUSE→OBS do portal.

Antes da entrega automática, preparar e revisar o manifesto de migração,
qualificar a resolução completa com e sem traduções e executar o fluxo real
de autorização, staging, offline, snapshot e recuperação. Em seguida vêm a
promoção revisada e os ensaios de instalação/atualização/rollback na ISO de
checksum exato. Desktop #125, #6/#102 e Updater #22 continuam abertos.

Nenhuma regra de migração é distribuída por esta documentação. O canal release
continua na 0.2.5; staging não habilita proteção parental nem altera o host.
Se houver regressão, bloquear a promoção, corrigir/reconstruir em staging e
repetir a qualificação do novo artefato.
