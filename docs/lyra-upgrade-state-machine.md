# Máquina de estados do Lyra Upgrade

Esta máquina é normativa para update e release-upgrade. Estados persistidos
usam os nomes abaixo e toda transição gera evento com sequência monotônica.

```text
Idle -> Checking -> Available -> Preflight -> Planned -> AwaitingConfirmation
                                                            |
                                                            v
Downloading -> Snapshotting -> ReadyToReboot ----------------+ (upgrade)
      |              |              |
      |              v              v
      +----------> Applying     ApplyingOffline
                         \         /
                          v       v
                       AwaitingReboot -> VerifyingBoot -> Completed

Estados de desvio: Blocked, Failed e NeedsRecovery.
```

Para update sem necessidade de reboot, `Applying` pode seguir diretamente para
`Completed` após verificação local. Release-upgrade sempre passa por
`ReadyToReboot`, `ApplyingOffline`, `AwaitingReboot` e `VerifyingBoot`.

| Estado | Escrita | Cancelável | Saídas normais |
|---|---:|---:|---|
| `Idle` | não | sim | `Checking` |
| `Checking` | cache autenticado | sim | `Available`, `Blocked` |
| `Available` | não | sim | `Preflight` |
| `Preflight` | não | sim | `Planned`, `Blocked` |
| `Planned` | estado/planos | sim | `AwaitingConfirmation` |
| `AwaitingConfirmation` | não | sim | `Downloading`, `Idle` |
| `Downloading` | cache RPM | sim, antes de pacote ativo | `Snapshotting`, `Blocked` |
| `Snapshotting` | snapshot/estado | não | `Applying`, `ReadyToReboot`, `Failed` |
| `Applying` | pacotes/sistema | somente fronteiras seguras | `AwaitingReboot`, `Completed`, `NeedsRecovery` |
| `ReadyToReboot` | estado/offline | não | `ApplyingOffline` |
| `ApplyingOffline` | sistema/repositórios | não | `AwaitingReboot`, `NeedsRecovery` |
| `AwaitingReboot` | não | não | `VerifyingBoot` |
| `VerifyingBoot` | resultado | não | `Completed`, `NeedsRecovery` |
| `Blocked` | evidência | sim | novo `Preflight` ou `Idle` |
| `Failed` | evidência | não | `Idle` após reconhecimento |
| `NeedsRecovery` | evidência | não | rollback explícito ou diagnóstico |
| `Completed` | resultado | não | terminal |

## Invariantes

1. Nenhuma alteração do sistema ocorre antes de confirmação e revalidação.
2. `Snapshotting` persiste o número do snapshot antes de avançar.
3. Depois de `Snapshotting`, nenhuma falha apaga automaticamente o snapshot.
4. Reinício ou queda da UI não muda o estado por si só.
5. Apenas o verificador empacotado pode emitir o resultado pós-boot.
6. Um estado terminal contém causa ou resultado; nunca somente “desconhecido”.
7. Uma operação por vez pode estar em `Snapshotting`, `Applying` ou
   `ApplyingOffline`.

## Matriz de falhas

| Falha | Estado/resultado obrigatório | Escrita permitida na retomada |
|---|---|---|
| rede indisponível antes do download | `Blocked`, cache válido apenas para exibição | nenhuma |
| manifesto inválido/expirado/replay | `Blocked` | nenhuma |
| pouco espaço, bateria ou Snapper ausente | `Blocked` | nenhuma |
| RPMDB ou repositório inconsistente | `Blocked` | nenhuma |
| plano mudou após confirmação | `Blocked`, exigir novo plano | nenhuma |
| autorização negada | `Failed` ou retorno a confirmação | nenhuma |
| lock do Zypper ocupado | `Blocked` | nenhuma |
| UI encerra durante download | operação continua ou pausa seguramente | somente cache |
| UI encerra durante aplicação | serviço continua; nova UI reconecta | conforme plano |
| serviço cai antes do snapshot | reconciliar e voltar a estado seguro | nenhuma escrita destrutiva |
| serviço cai após criar snapshot | validar snapshot persistido antes de continuar | somente etapa idempotente |
| energia cai durante aplicação | `NeedsRecovery` se conclusão não for comprovável | nenhuma repetição cega |
| `zypper` retorna 4 | `Failed`, preservar causa/snapshot | nenhuma |
| `zypper` retorna 102 | `AwaitingReboot` | persistir reboot requerido |
| `zypper` retorna 103 | `Failed` ou `NeedsRecovery` conforme escrita iniciada | preservar snapshot |
| `dracut` ou GRUB falha | `NeedsRecovery` | não marcar sucesso |
| boot seguinte falha | snapshot permanece selecionável | rollback explícito |
| boot inicia, verificação falha | `NeedsRecovery` | diagnóstico/rollback explícito |
| estado truncado, adulterado ou schema desconhecido | `Blocked`, alerta administrativo | nenhuma |
| falha ao limpar cache/log | preservar resultado primário e emitir warning | somente limpeza |

Cancelamento nunca envia sinal assíncrono para interromper RPM/libzypp no meio
de uma transação. A implementação deve injetar cada falha da matriz em testes
antes de promover a funcionalidade.
