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
deve usar libzypp/libsolv em leitura ou uma raiz de simulação isolada e gravável
pelo usuário. Seu resultado atravessa o contrato tipado de
[`schemas/lyra-upgrade-solver-v1.schema.json`](schemas/lyra-upgrade-solver-v1.schema.json).
Saída textual localizada nunca alimenta decisões.

## Versão inicial do protocolo

Requests e eventos usam `protocol_version: 1`. O envelope inicial aceita:

- `Inspect`: consulta fatos e bloqueios, sem Polkit;
- `PlanUpdate`: calcula update dentro da release, sem escrita;
- `PlanReleaseUpgrade`: exige manifesto autenticado e calcula migração;
- `Start`: referencia `operation_id`, `plan_sha256` e confirmação explícita;
- `Status`: consulta snapshot e eventos a partir de uma sequência;
- `Cancel`: pedido cooperativo, aceito somente em estados canceláveis;
- `AcknowledgeRecovery`: registra a escolha explícita após falha.

`Start`, `Cancel` e qualquer decisão de recuperação exigem autorização do
usuário ativo. O serviço associa a operação ao UID e à sessão que a criou; um
chamador não autorizado não lê inventário detalhado nem controla a operação.
Requests desconhecidos ou com campos adicionais não previstos falham.

O schema inicial está em
[`schemas/lyra-upgrade-protocol-v1.schema.json`](schemas/lyra-upgrade-protocol-v1.schema.json).
Schemas posteriores recebem arquivo novo; o v1 nunca muda de significado.

## Estado persistente

Cada transição incrementa `sequence` e persiste:

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

Ao iniciar, o serviço carrega somente diretórios regulares root-owned sob sua
raiz fixa. Uma operação em estado não terminal é reconciliada com fatos do
host antes de continuar. A ausência de evidência de conclusão nunca é tratada
como sucesso. Etapas repetíveis verificam seu efeito antes de repetir; etapas
não repetíveis transitam para `NeedsRecovery` quando o resultado é ambíguo.

## Suporte e compatibilidade

- somente Desktop `x86_64`, Btrfs/Snapper e releases declaradas são aceitos;
- update dentro da release não consulta nem altera rota de upgrade;
- schema desconhecido falha fechado e preserva estado/snapshot;
- a versão 1.0 deve entender o manifesto sucessor sem precisar atualizar antes
  o próprio Lyra Upgrade;
- remoção do pacote desativa suas unidades, mas nunca remove snapshots ou
  estado de recuperação automaticamente.
