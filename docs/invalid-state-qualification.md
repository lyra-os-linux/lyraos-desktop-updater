# Busca de operações pendentes com entradas inválidas

A auditoria #10 identificou que `pending_operation` retornava `None` no
primeiro erro de leitura, mesmo após encontrar um estado válido. A unit então
saía com código zero, sem verificar a atualização pendente.

O scanner agora continua por todas as entradas e distingue ausência de
operações de uma busca incompleta. Uma raiz inexistente/vazia é normal; uma
raiz inacessível ou que não seja diretório gera `POST_BOOT_STATE_SCAN_FAILED`
e saída 1. Estados válidos fora de AwaitingReboot/VerifyingBoot são ignorados.
A seleção mantém o candidato pendente mais recente; empates usam o ID em
ordem crescente, sem depender da ordem de leitura do diretório.

Entradas ilegíveis, nomes inválidos, estados sem arquivo ou que falham na
validação são diagnosticados com `POST_BOOT_STATE_ENTRY_INVALID`, nome escapado
e limitado e motivo estável. JSON e conteúdo dos estados não entram nos logs.
Links, FIFOs e arquivos especiais não são aceitos como diretórios de operação
ou arquivos de estado. O leitor mantém as validações de esquema, ID e conteúdo.

Se houver uma operação válida, ela é verificada normalmente. Seu resultado
pode ser Completed/Passed, enquanto a **unit termina com saída 1** por haver
outras entradas não resolvidas. O resumo `POST_BOOT_STATE_SCAN_INCOMPLETE`
explicita essa diferença. A sequência de manifesto só avança pela verificação
da operação válida; nunca se presume sucesso para o registro ilegível.

Se a própria operação pendente estiver corrompida e não houver candidato
válido, o verificador retorna erro sem executar os probes, criar um estado
substituto, avançar a sequência ou apagar/mover arquivos. Não é possível
construir um NeedsRecovery confiável sem ler ID, origem e demais dados. A
recuperação do registro exige diagnóstico administrativo; a falha do serviço
persiste nas execuções seguintes enquanto o problema não for resolvido.

## Reprodução em VM

Compilar sem executar o verificador na estação:

```sh
cargo build -p lyra-upgrade-verify --locked
python3 scripts/check-invalid-state-vm.py \
  --kernel /caminho/para/vmlinuz \
  --binary target/debug/lyra-upgrade-verify \
  --scenario mixed --log /tmp/lyra-invalid-state-mixed.log
python3 scripts/check-invalid-state-vm.py \
  --kernel /caminho/para/vmlinuz \
  --binary target/debug/lyra-upgrade-verify \
  --scenario corrupt --log /tmp/lyra-invalid-state-corrupt.log
```

Para reproduzir o defeito anterior, usar o binário do commit `c785212` com
`--scenario baseline`. A unit sai com sucesso, mas o estado válido permanece
AwaitingReboot junto de diretório vazio, JSON truncado, arquivo inesperado e
nome com quebra de linha. O candidato mixed deve concluir a operação válida,
manter os demais arquivos byte a byte, emitir quatro diagnósticos e falhar a
unit por busca incompleta. O candidato corrupt deve preservar o único estado
ilegível, sem sequência aplicada, e emitir um diagnóstico com saída 1.

O ensaio usa cold boot, systemd PID1, unit empacotada e verificador reais em
QEMU sem NIC, sem discos e sem contas do host. Os probes RPM/zypper/Btrfs/
Snapper/SecureBoot/GRUB são fixtures; não é qualificação de ISO ou firmware.
A semântica potencialmente mutável de `zypper verify` permanece pendente em
**#12 e impede a publicação do updater**. Não executar o verificador no host.

Os testes Rust complementam a VM com 24 ordens de criação, seleção determinística,
estados concluídos, esquema/identidade/conteúdo inválidos, nomes não UTF-8,
links, FIFO, raiz ausente/vazia/inválida e preservação dos arquivos.

## Qualificação de 09/09/2026

Os três cold boots passaram com kernel `6.12.0-160100.4-default`: baseline
`c785212` reproduziu a omissão da operação; candidato mixed concluiu a operação
válida com quatro diagnósticos e estado dos vizinhos preservado; candidato
corrupt preservou o registro ilegível, sem sequência aplicada nem estado novo.
Nos dois cenários corrigidos a unit retornou 1, conforme o contrato de busca
incompleta. Também passaram 88 testes Rust, 28 Python, fmt e Clippy.
