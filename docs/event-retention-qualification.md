# Histórico persistido e retenção de eventos

A correção da auditoria #11 mantém `events.jsonl` legível quando o histórico
atinge 8 MiB. O tamanho considerado inclui o JSON serializado e a quebra de
linha; registros acima de 16 KiB são recusados antes de alterar o arquivo.
Ao ultrapassar 8 MiB, o serviço descarta apenas as linhas técnicas mais antigas,
com alvo de aproximadamente 4 MiB para evitar compactação a cada gravação.
As mudanças de estado já persistidas são preservadas.

A substituição usa arquivo temporário de modo 0600 no mesmo diretório,
sincronização de dados, rename atômico e sincronização do diretório. Leitores e
escritores compartilham um `flock` em `.events.lock`, cujo inode não muda com a
compactação. Arquivos, locks e diretório da operação recusam links simbólicos;
arquivos e locks também recusam hard links e arquivos especiais.

O aviso `history-truncated` tem sequência reservada 0 e guarda o total acumulado
de linhas removidas (`lines`) e a maior sequência da compactação (`high_sequence`).
Ele é retornado mesmo em consultas incrementais. A interface substitui avisos
pela combinação sequência 0 + identificador, sem apagar eventos normais nem
avançar o cursor. O serviço restaura a sequência máxima do disco, inclusive
quando a compactação removeu a linha técnica mais recente. Registros normais
continuam com sequência positiva e crescente. O formato das mensagens e a
versão 2 do protocolo são mantidos; os novos avisos têm tradução pt-BR/en-US/es-ES.

## Histórico incompleto e falha de escrita

O leitor aceita o pequeno excesso legado de até 8 MiB + 16 KiB e o compacta na
próxima gravação. Limita o tamanho total e o de cada registro durante a leitura.
Um registro parcial, inválido, com ID diferente, sequência regressiva ou leitura
interrompida preserva o prefixo válido e produz `EVENT_LOG_READ_FAILED`, inclusive
quando existe cache em RAM. Não se reescreve automaticamente um histórico
corrompido. Uma operação planejada que ainda não gravou eventos pode ter histórico
vazio sem erro.

Falhas de append são sinalizadas em RAM, no stderr do serviço e, quando o sistema
de arquivos permite, no marcador persistente `events.error`. O Status expõe
`EVENT_LOG_WRITE_FAILED` e os detalhes ainda disponíveis. Se a operação já tem
seu próprio erro, ele conserva prioridade; o problema do histórico também aparece
como aviso separado nos detalhes. Consultar o histórico não altera estado,
snapshot, sequência ou resultado da operação. A interface não sugere que a
atualização falhou apenas porque faltaram detalhes.

Uma falha de logging não interrompe uma transação de pacotes em andamento.
Iniciar uma operação com histórico já incompleto ou marcador de falha é recusado.
O marcador é conservado para que uma gravação posterior bem-sucedida não esconda
a lacuna anterior. Se nem o marcador puder ser gravado (por exemplo, disco cheio),
o diagnóstico depende da instância ativa e da captura de stderr; não há promessa
de recuperação de dados que nunca chegaram ao armazenamento.

Se apenas os eventos normativos esgotarem os 8 MiB, a nova gravação é recusada
explicitamente e o arquivo anterior permanece intacto. Não há descarte silencioso
de mudanças de estado nem criação de arquivos de arquivo morto sem limite.

## Validação

```sh
cargo test --workspace --exclude lyra-upgrade-ui --locked
cargo fmt --all --check
cargo clippy --workspace --exclude lyra-upgrade-ui --locked --all-targets --all-features -- -D warnings
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests -v
```

Os testes usam as funções reais do serviço e arquivos temporários: limite exato
8 MiB, overflow legado, três compactações sucessivas, retenção normativa,
contagem acumulada, limite serializado incluindo escapes/quebra de linha,
registro parcial/inválido, entrada excessiva, links/FIFO, capacidade normativa
esgotada, sequência após descarte do registro mais recente e leitura concorrente
à substituição atômica. Um subprocesso separado reabre o histórico compactado e
persiste a próxima sequência.

Testes de Status recriam o serviço sem cache, provocam falha real de escrita nos
dois emissores, conferem persistência do aviso, round-trip JSON e preservação do
erro/estado da operação. O teste da interface executa o JavaScript real em Node
nos três idiomas, verifica os avisos renderizados, deduplicação e cursor.
Nenhuma atualização, serviço privilegiado, pacote instalado ou estado de boot
do host é usado. A qualificação RPM e a publicação permanecem etapas posteriores.
