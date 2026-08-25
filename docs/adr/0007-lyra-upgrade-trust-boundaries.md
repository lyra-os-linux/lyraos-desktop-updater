# ADR 0007: fronteiras, protocolo e persistência do Lyra Upgrade

- Estado: aceita
- Data: 2026-08-18
- Proprietário: `lyraos-desktop-updater`
- Escopo: produto
- Origem: migrada de `lyraos-desktop` em 2026-08-24
- Issue histórica: `lyraos-desktop#81`

## Contexto

O Desktop precisa atualizar pacotes dentro de uma release e, separadamente,
migrar entre releases homologadas. As duas operações atravessam reinícios e
podem modificar repositórios, RPMs, initramfs e boot. A interface gráfica e o
usuário chamador não são fronteiras de confiança; uma autorização Polkit não
torna confiável o request recebido pelo serviço root.

O protocolo JSON Lines do instalador é adequado a uma execução contida na
sessão live, mas não oferece descoberta, reconexão ou persistência suficientes
para uma operação que deve sobreviver à queda da UI, do serviço ou da energia.

## Decisão

### Operações distintas

`UpdateWithinRelease` conserva os repositórios e a identidade da release e usa
a semântica de `zypper update`. `ReleaseUpgrade` aceita somente um par
origem/destino explicitamente homologado por manifesto assinado e usa uma
transação offline equivalente a `zypper dup`. Uma operação nunca é promovida
implicitamente à outra.

### Fronteiras de processo

- cliente, interface, descoberta e planejamento executam sem root;
- um serviço systemd root é o único executor e mantém o estado canônico;
- Polkit autoriza ações estreitas por operação, sem autorização genérica;
- o serviço aceita somente requests tipados pelo schema publicado, reobtém os
  fatos do host e recalcula o plano antes da primeira escrita;
- a UI nunca fornece shell, `argv`, URL de repositório, nome de binário ou
  caminho de arquivo para execução privilegiada;
- binários, argumentos, diretórios de estado e endpoints de manifesto são
  definidos pelo pacote ou por manifesto assinado validado pelo serviço.

O transporte inicial será uma API local versionada, adequada à reconexão e à
consulta do estado. A escolha concreta entre D-Bus e socket Unix pertence à
implementação histórica `lyraos-desktop#103`; ela não pode alterar os tipos
normativos nem reduzir a autorização por chamador definida aqui.

### Confiança no manifesto

TLS protege transporte, mas não autoriza upgrade. Índice e manifesto precisam
de assinatura válida pela chave de release, versão de schema conhecida,
edição/arquitetura corretas, origem e destino explícitos, validade temporal e
identificador monotônico. Cache só pode ser usado enquanto assinado e válido.
Manifesto expirado, retirado, desconhecido, repetido ou que implique downgrade
não autorizado falha antes de qualquer escrita.

### Plano e confirmação

O core produz um plano canônico e serializável a partir de fatos observados e
política autenticada. O usuário confirma o hash do plano. Imediatamente antes
de executar, o serviço redescobre o host, revalida o manifesto e exige o mesmo
hash. Divergência volta a `Blocked`; não há coerção ou confirmação presumida.

### Estado durável

O estado canônico fica em `/var/lib/lyra-upgrade/operations/`, propriedade de
root e modo `0700`; cada operação usa um UUID gerado pelo serviço. Arquivos são
abertos sem seguir links, escritos em arquivo temporário no mesmo filesystem,
sincronizados, renomeados atomicamente e seguidos de `fsync` do diretório.
Versão de schema desconhecida, permissões inseguras, proprietário incorreto,
truncamento ou conteúdo incoerente bloqueiam retomada.

O estado registra apenas identificadores e fatos necessários: operação,
transição, hash do plano/manifesto, snapshot, versões, resultado e códigos de
erro. Não registra tokens, credenciais, conteúdo de configuração pessoal ou
inventário de `/home`.

A ESP não recebe o estado geral. Se o verificador de boot não conseguir ler a
raiz anterior, pode existir um marcador mínimo autenticado, criado pelo
serviço, contendo apenas UUID, versão de schema, snapshot e hash do estado. O
fluxo inicial deve preferir `/var`; uso da ESP exige teste específico e não é
pré-condição para update dentro da mesma release.

### Snapshots e recuperação

Antes da primeira alteração, o serviço cria snapshot Snapper somente leitura e
persiste seu número. O snapshot protege o sistema, não substitui backup de
`/home`. A operação só termina após verificação pós-boot. Falha preserva o
snapshot e resulta em `NeedsRecovery`; rollback não é disparado silenciosamente
após um boot funcional.

O escopo inicial é exclusivamente Lyra OS Desktop com raiz Btrfs, Snapper e
boot UEFI suportado. Lyra OS Server 27.02 em ext4 é recusado.

### Logs

Eventos estruturados ficam no journal e no estado restrito. A exportação para
`lyra-report` é explícita, sanitizada e exclui URLs com credenciais, tokens,
chaves, conteúdo de arquivos, nomes de usuário e caminhos pessoais. O padrão é
retenção das três operações mais recentes por 90 dias; estado de operação
ativa, falha não resolvida e snapshot associado não é removido automaticamente.

## Consequências

- fechamento da UI não cancela nem perde a operação;
- o serviço tem mais complexidade que o protocolo efêmero do instalador;
- formatos persistentes e protocolo precisam de compatibilidade explícita;
- nenhuma versão pode ser publicada se uma falha injetada deixar estado
  ambíguo, perder o snapshot conhecido ou repetir cegamente escrita;
- o mecanismo pode ser retirado da imagem antes da Beta 1 sem afetar Zypper e
  Snapper, que continuam disponíveis como componentes da base openSUSE.

## Alternativas rejeitadas

- executar Zypper pela UI com `pkexec`: amplia a fronteira privilegiada e não
  sobrevive à UI;
- aceitar comandos ou URLs do manifesto/UI: permite injeção e confunde dados
  não confiáveis com política;
- confiar apenas em HTTPS ou no maior número de versão encontrado: não protege
  contra replay, retirada ou rota não homologada;
- guardar todo o estado na ESP: expõe dados e transforma uma partição de boot
  limitada na base transacional do produto;
- atualizar silenciosamente: contraria confirmação, previsibilidade e
  recuperação explícita do Lyra.
