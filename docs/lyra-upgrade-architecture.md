# Arquitetura e contratos do Lyra Upgrade

Este documento detalha a decisão normativa da
[ADR 0007](adr/0007-lyra-upgrade-trust-boundaries.md). A implementação deve
rejeitar qualquer campo ou transição que não pertença à versão de protocolo
suportada.

## Componentes e confiança

```text
índice/manifesto assinado ──> core sem root ──> plano + hash ──> UI
                                    │                           │
                                    └──── request tipado ───────┘
                                                  │ Polkit
                                                  v
                                      serviço root persistente
                                      │ revalida fatos/política
                                      │ snapshot + Zypper
                                      v
                                      verificador pós-boot
```

O core pode observar o host e simular, mas não altera repositórios, pacotes,
snapshots ou boot. A UI apresenta fatos e solicita uma operação; não interpreta
texto localizado do Zypper como decisão. O serviço é responsável por política,
concorrência, revalidação e execução. O verificador pós-boot só conclui uma
operação previamente registrada.

O executável `zypper` do Leap exige root mesmo para `update --dry-run`; portanto
ele não é usado pelo core como atalho privilegiado. A implementação do solver
usa uma cópia temporária do RPMDB, configuração e cache zypp. `unshare --user
--map-root-user` fornece UID 0 somente dentro do namespace para `zypper --root`.
A indisponibilidade de namespaces bloqueia o plano; não abre Polkit como fallback. Seu resultado atravessa o contrato tipado de
[`schemas/lyra-upgrade-solver-v1.schema.json`](schemas/lyra-upgrade-solver-v1.schema.json).
Saída textual localizada nunca alimenta decisões.

## Protocolo atual (v3)

Requests usam `protocol_version: 3`; respostas e eventos pertencem à mesma conexão tipada. O envelope aceita:

- `CheckRelease`: obtém uma oferta assinada ou cache ainda válido para exibição;
- `ReadTrustState`: lê o contador antirreplay pelo broker protegido;
- `Inspect`: consulta fatos e bloqueios, sem Polkit;
- `PlanUpdate`: calcula update dentro da release, sem escrita;
- `PlanReleaseUpgrade`: exige manifesto autenticado e calcula migração;
- `Start`: referencia `operation_id`, `plan_sha256` e confirmação explícita;
- `Status`: consulta snapshot e eventos a partir de uma sequência;
- `Cancel`: pedido cooperativo, aceito somente em estados canceláveis;
- `AcknowledgeRecovery`: registra a escolha explícita após falha.

`Start`, `Cancel`, `Rollback` e `KeepCurrent` exigem autorização do
usuário ativo. `ShowDiagnostics` somente consulta. O serviço associa a operação ao UID
obtido de Polkit; o broker de consulta obtém o UID por `SO_PEERCRED`. Um
chamador não autorizado não lê inventário detalhado nem controla a operação.
Requests desconhecidos ou com campos adicionais não previstos falham.

O schema atual está em
[`schemas/lyra-upgrade-protocol-v3.schema.json`](schemas/lyra-upgrade-protocol-v3.schema.json).
O arquivo v1 permanece histórico. Plano e inventário completo seguem
[`schemas/lyra-upgrade-plan-v3.schema.json`](schemas/lyra-upgrade-plan-v3.schema.json).

O manifesto sucessor assinado obedece ao contrato público
[`schemas/lyra-upgrade-release-manifest-v1.schema.json`](schemas/lyra-upgrade-release-manifest-v1.schema.json).
Além da rota exata entre releases, ele fixa a sequência anti-replay, validade,
versão mínima do atualizador, piso de espaço livre, repositórios, fingerprints e
exceções explícitas do solver. Campos ou versões de schema desconhecidos são
rejeitados antes de qualquer escrita no sistema.
Schemas posteriores recebem arquivo novo; o v1 nunca muda de significado.

O canal instalado é `stable`, em `/etc/lyra-upgrade/channel`. Somente o valor
administrativo explícito `testing` permite manifestos com status `testing` ou
destino semanticamente pré-release; arquivo ausente mantém `stable` e qualquer
outro conteúdo bloqueia a descoberta. Manifestos `paused` e `withdrawn`
permanecem indisponíveis em ambos os canais.

O manifesto externo é produzido por `scripts/release-manifest.py`. A ferramenta
rejeita campos desconhecidos, rotas inseguras, janelas de validade invertidas,
políticas incompletas e fingerprints abreviados; em seguida grava
`releases-v1.json` de forma canônica, sem sobrescrever artefatos existentes, e
opcionalmente gera a assinatura destacada `releases-v1.json.asc` com uma chave
indicada pelo fingerprint completo. A chave privada e o destino de publicação
nunca fazem parte da imagem instalada.

O canal `stable` usa exclusivamente as URLs compiladas no serviço. Quando um
administrador opta explicitamente por `testing`, ele também deve criar
`/etc/lyra-upgrade/testing-manifest-base-url` com uma única URL-base HTTPS,
terminada em `/`, sem credenciais, query ou fragmento. Somente nesse canal o
serviço deriva `releases-v1.json` e `releases-v1.json.asc` dessa origem. Isso
permite ensaios externos sem substituir ou flexibilizar o endpoint estável.

`scripts/signing-handoff.py` prepara a transferência para o ambiente autorizado
sem material secreto: manifesto canônico, SHA-256 e pedido que fixa o
fingerprint `01B63EEDBE6B079126A0116EFA7353A131ECEFEB`. O retorno deve conter
somente a assinatura destacada correspondente; a chave privada nunca é copiada
para o host de desenvolvimento.

A identidade usada para autorizar e verificar a rota vem de
`/usr/lib/lyra-os/product-release`, pertencente ao RPM `lyra-release`. Versão,
arquitetura e build ID formam uma única identidade transacionável. Os
metadados completos da composição permanecem em `/usr/lib/lyra-os/release` e
não são usados como identidade atualizável. Assim, o solver inclui a mudança de
release no plano e o `zypper dup` consegue instalar a identidade sucessora
antes da verificação pós-boot.

## Estado persistente

Consultas e planos são transitórios. Somente `Start` autenticado, após recalcular
e comparar o plano, cria estado protegido. Cada transição persistida incrementa `sequence`:

- versão do schema e UUID;
- tipo da operação e estado atual;
- release/build de origem e destino, quando houver;
- hashes SHA-256 do plano e manifesto;
- último passo confirmado, bloqueio ou erro estruturado;
- número do snapshot somente depois de criado e sincronizado;
- instantes de criação e atualização em UTC;
- resultado da verificação pós-boot.

O schema inicial está em
[`schemas/lyra-upgrade-state-v1.schema.json`](schemas/lyra-upgrade-state-v1.schema.json).
Plano e inventário completos ficam em arquivos separados, root-only, ligados
por hash. A UI recebe uma projeção sanitizada.

## Contrato do solver

O resultado do solver enumera cada instalação, atualização, downgrade,
remoção ou reinstalação com versão, arquitetura, vendor, repositório e tamanhos
anterior/proposto. Também comprova quais metadados de repositório foram
validados e informa problemas sem tentar resolvê-los agressivamente.

O preflight bloqueia schema desconhecido, problema do solver, downgrade,
remoção fora da allowlist, troca de vendor não autorizada e atualização parcial
de um grupo lockstep. Downloads, crescimento da transação, estimativa do
snapshot e margem conservadora compõem o requisito de espaço. O plano inclui a
lista ordenada de mudanças; por isso seu SHA-256 muda diante de qualquer
alteração da resolução.

As listas auxiliares `to-change-vendor` do zypper são incorporadas sem duplicar
um upgrade/downgrade/reinstall. A contagem RPM deve coincidir com
`packages-to-change`; entradas incompletas e mudanças de arquitetura bloqueiam
a interpretação. O contrato existente representa uma troca isolada como
`Reinstall` com a mesma versão e fornecedores distintos.

A identificação usa cabeçalhos do banco RPM para a versão instalada e
`rpm:vendor` do RPM-MD para o candidato exato (nome, epoch/versão/release,
arquitetura, alias). O cache deve ser o mesmo utilizado pelo zypper após
validação das assinaturas; o checksum SHA-256/SHA-512 de primary também é
conferido contra repomd. XML simples, gzip, xz e zstd são suportados.
Metadados incompletos, identidades ambíguas/desconhecidas e formatos não
suportados bloqueiam, sem inferir fornecedor a partir do alias.

A mesma allowlist exata e direcional vale no planejamento, antes/depois do
download em staging e na revalidação offline. Manifesto e identidades dos
pacotes continuam vinculados ao hash confirmado. O espaço restante é
reavaliado em cada fase, mas a redução dos bytes a baixar após preencher o
cache não altera a identidade da transação aprovada.

Na atualização de versão, solver e inventário usam o contexto de destino:
`repos.d`, `cache`, `cache/raw`, `cache/solv` e `cache/packages` da operação.
Isso inclui `packages --orphaned`, que precisa dos metadados mesmo com
`--no-refresh`. O planejamento temporário usa a mesma disposição de diretórios;
staging verifica novamente o plano após preparar esse contexto. Mensagens de
progresso não são identidades de pacotes. Falhas de inventário bloqueiam a
operação, e mudanças nos nomes de bloqueios/órfãos alteram o plano.

O serviço offline usa `PrivateNetwork=yes`. Revalida e aplica os RPMs com o
contexto preparado; só após uma aplicação bem-sucedida (código 0 ou 102)
publica os repositórios de destino, preservando uma cópia do conjunto anterior.
Assim, falhas de descoberta, metadados, preflight ou download de RPM ausente
não provocam a troca antecipada de repositórios pelo atualizador. O código 103
continua sendo falha. Se a aplicação de RPMs já alterou o sistema, incluindo
arquivos de repositório escritos por pacotes, não há rollback automático:
`NeedsRecovery` preserva o snapshot para recuperação explícita.

[A qualificação em VM](offline-cache-qualification.md) cobre essa fronteira.
Boot completo, Secure Boot e reconstrução real de initramfs/GRUB continuam
sendo gates independentes.

### Fatos vinculados ao plano de versão

| Entrada | Origem e regra entre planejamento e boot offline |
| --- | --- |
| Release de origem, filesystem e Secure Boot | Host instalado; mudanças alteram o plano. |
| Release de destino e política | Manifesto confirmado, incluindo seu hash e piso de espaço. |
| Repositórios e aliases válidos | Contexto preparado de destino em todas as fases. Repositórios ativos da origem, inclusive terceiros desabilitados, não são usados pelo solver de destino. |
| Bloqueios e órfãos | Bloqueios locais e inventário consultado com os repositórios de destino; mensagens de progresso não participam. |
| Versão, arquitetura, fornecedor e ação dos RPMs | Solver atual comparado ao plano confirmado; mudanças reais continuam recusadas. |
| Espaço disponível, bateria, RPMDB e Snapper | Preflight atualizado a cada fase; não são congelados para liberar execução. |
| `required_bytes` | A estimativa atual precisa caber no disco e respeitar o piso do manifesto. Só depois disso o hash reutiliza o valor confirmado, pois o download já em cache reduz a necessidade restante. |

O conjunto ativo da origem é preservado até a aplicação e guardado no backup
ao publicar o destino. A VM exercita aliases de origem/destino diferentes,
terceiros desabilitados e um piso de espaço maior que a estimativa do solver;
o teste Rust recusa espaço um byte abaixo do piso e mudança de versão do RPM.

## Política de comandos privilegiados

O executor contém operações fechadas, equivalentes a:

- atualizar/baixar metadados dos repositórios já aprovados;
- baixar os RPMs exatos do plano;
- criar e consultar snapshot Snapper da configuração `root`;
- executar `zypper update` ou a transação offline `dup` conforme o tipo;
- executar `dracut` e regenerar GRUB somente quando o plano exigir;
- instalar/remover unidades de transação offline empacotadas.

Cada operação monta internamente o `argv` a partir de enums e identificadores
validados. Não existe operação `Run`, campo `command`, caminho arbitrário ou
script vindo do manifesto. Somente um lock global pode atravessar `Applying`.

## Retomada

A UI retoma consultas sem autenticação. Fechar a janela não encerra os workers.
No próximo boot, o verificador adquire o lock global e reconcilia operações:
`Downloading` interrompido vira `Failed`; `Snapshotting`, `Applying` e
`ApplyingOffline` interrompidos viram `NeedsRecovery`. `ReadyToReboot` sem o
marcador esperado também exige recuperação. Não repete a aplicação. Uma queda
do próprio serviço durante a mesma sessão pode deixar o último estado visível
até essa reconciliação no próximo boot; isso não é uma prova de atividade nem
uma autorização para reiniciar a transação.

O broker `lyra-upgrade-query.socket` responde somente `Status` e `ReadTrustState`.
Ele não executa solver, rede, comandos administrativos ou escritas, inclusive
criação de locks de histórico. Valida UID do socket e mantém diretórios privados.
A UI usa um processo `--read-only` para consultas e só inicia `pkexec` ao receber
uma ação administrativa explícita. Limites: request 4 MiB, resposta 16 MiB,
conexão de consulta 10 s, unidade 15 s e 16 conexões concorrentes.

O cache da oferta pertence ao usuário e serve apenas para exibição. Assinatura,
rota, sequência e validade são verificadas novamente na leitura. Planejamento,
confirmação e staging exigem oferta atual obtida pela rede. O offline usa os
bytes assinados retidos, revalida com a chave empacotada e recusa expiração.
A validade é o intervalo `[valid_from, valid_until)`.

Inventários completos antes/depois incluem versões, arquiteturas e vendors de
RPMs e os caminhos `.rpmnew`/`.rpmsave` de `/etc`, sem ler configurações nem
arquivos pessoais. A conclusão compara pacotes mantidos e propostos; versões
antigas de pacotes atualizados podem permanecer pela política multiversion.
Rollback compara o inventário de origem. Snapshot do sistema não é backup
pessoal: o plano exige `/home` em outro dispositivo ou subvolume, e a UI exige
reconhecimento do aviso de backup antes de uma migração de versão.

## Suporte e compatibilidade

- somente Desktop `x86_64`, Btrfs/Snapper e releases declaradas são aceitos;
- update dentro da release não consulta nem altera rota de upgrade;
- schema desconhecido falha fechado e preserva estado/snapshot;
- a versão 1.1 deve entender o manifesto sucessor sem precisar atualizar antes
  o próprio Lyra Upgrade;
- remoção do pacote desativa suas unidades, mas nunca remove snapshots ou
  estado de recuperação automaticamente.

## Aceite e qualificação

A [matriz de aceite Desktop #13](desktop-13-acceptance.md) concilia implementação,
testes e gates externos. Os documentos de VM registram qualificações históricas
de versões anteriores; não aprovam automaticamente o protocolo/plano v3.
