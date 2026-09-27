# Consulta Snapper sem privilégios — 0.2.8

O ensaio do portal com o RPM 0.2.7 encontrou `SNAPPER_UNAVAILABLE` no
planejamento feito por uma conta comum. `snapper --no-dbus --config root
get-config` devolvia `Operation not permitted`, embora a consulta root e a
criação/leitura/remoção de snapshot funcionassem. A alternativa D-Bus executada
pela própria conta também retornava `No permissions` com a configuração
padrão, sem `ALLOW_USERS` ou `ALLOW_GROUPS`.

A descoberta do serviço, do solver e do CLI agora consulta apenas a disponibilidade
da configuração root pelo broker de leitura existente. A nova mensagem tipada
`ReadRecoveryReadiness` não aceita caminho, configuração, comandos ou UUID de
operação; a resposta contém somente um booleano e o identificador da consulta.
O cliente verifica a identidade root do peer, como nas consultas existentes.

O broker executa o comando fixo `snapper --config root get-config` com limite
de cinco segundos. O snapperd nativo faz a inspeção privilegiada; não houve
ampliação de capabilities, filtros systemd ou permissão de usuários no Snapper.
Erro, timeout, resposta incompatível e broker indisponível mantêm a recusa.
Antes da execução autenticada, o planner root continua refazendo a consulta
local direta e o plano completo. O protocolo v3 recebe variantes aditivas;
brokers antigos recusam a consulta e não liberam o planejamento.

## Validação

- 147 testes Rust, 35 contratos Python/interface, fmt e Clippy.
- `tests/snapper_readiness_vm.py`: broker systemd real, disponível → indisponível
  com snapperd mascarado temporariamente → restaurado, com acesso direto da
  conta comum ainda recusado.
- Ensaio feito na VM Btrfs descartável do portal, em 27/09/2026. Para o primeiro
  teste nativo, apenas o executável do serviço foi substituído temporariamente
  por compilação local da correção sobre 0.2.7. O binário original foi preservado;
  isso não qualifica um RPM 0.2.8 do OBS.

O planejamento assinado completo, a autenticação Polkit, a execução offline,
o boot/rollback e a publicação do RPM 0.2.8 permanecem gates distintos.
O canal release continua em 0.2.5. A correção não altera o manifesto assinado
nem conclui a entrega do portal ou a qualificação da ISO.
