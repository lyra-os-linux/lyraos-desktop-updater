# Desktop #13 — aceite de implementação do Lyra Upgrade

Data: 2026-09-15. Escopo: GNOME, RPM, Lyra Upgrade 0.2.4; protocolo e plano v3.
Issue: https://github.com/lyra-os-linux/lyraos-desktop/issues/13.
Esta matriz concilia a implementação com os critérios históricos; não libera
uma ISO nem substitui a prova de migração entre releases publicadas.

## Matriz

| Critério | Implementação e evidência reproduzível | Aceite |
| --- | --- | --- |
| Consultar, planejar e acompanhar sem senha | `--read-only`, namespace de usuário e RPMDB temporário; `Request::needs_authorization`, testes do broker por UID e `tests/release_flow.mjs` | Implementado; sem fallback privilegiado para consultas |
| Autorização antes de persistir ou alterar sistema | `Start` reobtém/recalcula plano, verifica igualdade/hash e só então grava; `Cancel`, `Rollback`, `KeepCurrent` administrativos | Implementado; negativa de Polkit conserva a revisão |
| API fechada/versionada | Protocolo v3, campos desconhecidos rejeitados, limites de mensagens, sem argv/caminhos de execução do cliente | Implementado; schemas v1 preservados |
| Oferta autenticada | GPGV com chave empacotada, identidade completa de origem/destino, canal, versão mínima, sequência, janela de validade | Implementado; testes de assinatura real, adulteração, expiração e replay |
| Descoberta e cache | Verificação manual/ao abrir e a cada seis horas enquanto a UI está aberta; cache do usuário reautenticado só para exibição | Implementado; sem daemon de descoberta ou telemetria |
| Repositórios de destino | HTTPS, prioridade, fingerprint primário único, GPG obrigatório; conjunto do manifesto e allowlists de remoção/vendor | Implementado; conjunto anterior preservado antes de publicar o destino |
| Inventário | Todos os RPMs instalados, versões/arquiteturas/vendors, locks, órfãos, aliases/estado dos repositórios; hash do plano inclui pacotes mantidos | Implementado; manifesto registra URLs, prioridades e fingerprints do destino |
| Preflight | Identidade Desktop x86_64, Btrfs/Snapper, RPMDB/lock, espaço raiz/boot/ESP, `/home` distinto, energia/Secure Boot | Implementado; pisos adicionais de 256 MiB no boot e 32 MiB na ESP; ausência de fatos bloqueia |
| Revisão pelo usuário | Origem/destino, ações/versões/vendors, repositórios, espaço, reinício, aviso de backup; pt-BR/en-US/es-ES | Implementado; reconhecimento do aviso antes de migrar versão |
| Download e staging | Revalidação antes/depois do download; snapshot RO persistido antes da aplicação; bytes/assinatura retidos e marcador sincronizado | Implementado; testes do plano e das fronteiras de persistência |
| Aplicação offline | Sem rede, plano e manifesto autenticados novamente, mesma rota/validade, conjunto preparado de RPMs e repos | Implementado; repetir qualificação integrada com artefatos v3 em #24/#26 |
| Preservação e diagnóstico | Inventário antes/depois; pacotes mantidos e propostos conferidos, remoções não previstas impedem sucesso; caminhos `.rpmnew`/`.rpmsave` em `/etc` | Implementado; não lê conteúdo das configurações nem arquivos pessoais |
| Reinício/verificação | Identidade incluindo build, RPMDB, dependências, unidades/target, GRUB, inventário e contador antirreplay | Implementado; boot real da composição GNOME permanece gate |
| Queda da UI/serviço | Fechar UI mantém workers; consultas retomam sem senha. Boot reconcilia download interrompido como falha e aplicação/snapshot ambíguos como recuperação | Implementado; não repete transações após queda |
| Rollback explícito | Intenção/clones duráveis, seleção Snapper, conferência de origem/clone/inventário no boot; nunca automático | Implementado; boot/GRUB e preservação real do home permanecem gate |
| Logs e relatório | Estado privado por UID, histórico limitado/sanitizado, inventários privados, exportação explícita da UI | Implementado; consulta não cria arquivos/locks |

## Limites e compatibilidade

- Snapshot do sistema não substitui backup pessoal. O preflight aceita `/home`
  em outro dispositivo ou subvolume Btrfs distinto; um bind do mesmo subvolume
  da raiz não passa. Isso não certifica integridade física do disco.
- Namespace de usuário desabilitado impede planejamento sem privilégios; o
  produto informa bloqueio em vez de pedir senha para abrir/consultar.
- A queda do serviço na mesma sessão pode manter o último estado mostrado até
  a reconciliação no próximo boot. Não há retomada automática da aplicação.
- Versões antigas de um pacote atualizado podem permanecer instaladas para
  suportar kernels multiversion. Pacotes novos inesperados ou mantidos ausentes
  impedem a conclusão bem-sucedida.
- A descoberta usa a identidade e a rota assinada por release, sem inferir que
  toda composição openSUSE intermediária é uma release Lyra suportada.
- Planos antigos precisam ser refeitos. Um staging antigo sem os bytes
  assinados/inventário v3 é recusado; preservar estado e snapshot para diagnóstico
  e recuperação. Não atualizar o updater durante uma operação já preparada.
- As qualificações VM antigas permanecem evidência histórica de suas revisões;
  não aprovam os novos artefatos nem dispensam assinatura/offline/preflight.

## Validação e gates

Testes locais:

```sh
cargo fmt --all --check
cargo test --workspace --exclude lyra-upgrade-ui --locked --offline
cargo clippy --workspace --exclude lyra-upgrade-ui --all-targets --all-features --locked --offline -- -D warnings
python3 -m unittest discover -s tests -v
cargo run --offline -p lyra-upgrade-service --example inspect-plan
```

O último comando apenas calcula um plano em cópia descartável; não executa
`Start` nem instala pacotes no host. Os testes de assinatura precisam de GPG
funcional, inclusive seus sockets locais. A compilação completa inclui a UI.

Permanecem abertos os gates externos:

1. [Desktop #26](https://github.com/lyra-os-linux/lyraos-desktop/issues/26):
   sucessor GNOME realmente publicado/assinado, rota e baseline corretos,
   sem substituir artificialmente o updater da versão de origem.
2. [Desktop #24](https://github.com/lyra-os-linux/lyraos-desktop/issues/24):
   update/reboot/offline/rollback na mesma VM, falhas de rede/disco/processo/RPM/
   initramfs/estado, preservação real dos dados, variantes padrão/NVIDIA e
   Secure Boot. Remover VMs e discos depois dos ensaios.
3. Execução desses ensaios após as issues de implementação e a auditoria
   Desktop #78; então testar as duas ISOs GNOME antes de lançar Alpha 8.

O fechamento integral de #13 depende dessas evidências. Código e testes locais
não transformam os gates pendentes em aprovação de release.
