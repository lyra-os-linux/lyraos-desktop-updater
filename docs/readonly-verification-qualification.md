# Verificação pós-boot sem reparo de pacotes

A auditoria #12 identificou uma escrita fora do plano: `zypper verify` pode
reparar dependências durante VerifyingBoot. Essa fase não autoriza transações.
A correção usa sempre:

```text
/usr/bin/zypper --xmlout --non-interactive --no-refresh verify --dry-run --details
```

`--dry-run` impede a aplicação; o código de saída sozinho não confirma saúde.
O XML precisa conter exatamente um `install-summary` diretamente em `stream`,
sem ações e com `packages-to-change`, download e alterações de espaço zerados.
Propostas de instalação, remoção ou qualquer outra alteração, prompts do solver,
mensagens de erro, códigos não zero, saída vazia/incompleta ou resumo inválido
resultam em `POST_BOOT_DEPENDENCIES_FAILED`, NeedsRecovery/Failed e saída 1 da
unit. Não se avança a sequência do manifesto nessa situação. Não há fallback
para uma execução mutável, seleção automática de solução ou botão de reparo
implícito: qualquer reparo requer outra operação com plano e autorização próprios.

O parser não depende de mensagens traduzidas. Limita a captura a 1 MiB e a
profundidade XML a 32, recusa DTD e resumos duplicados/mal posicionados e falha
diante de propostas contraditórias com contagem zero. O timeout de 180 segundos
do supervisor continua cobrindo os subprocessos, e a saída técnica do zypper
não é copiada para o diagnóstico público. O pacote usa quick-xml 0.38.4, já
presente no lockfile do serviço, sem nova família de dependências.

Referências upstream: [manual do zypper](https://manpages.opensuse.org/Leap-16.0/zypper/zypper.8.en.html)
e [formato do resumo XML](https://github.com/openSUSE/zypper/blob/master/src/output/xmlout.rnc).
O comportamento foi conferido também no código de `Summary::dumpAsXmlTo` e
na saída do zypper instalado, capturada exclusivamente dentro da VM.

## Reprodução

Compilar sem executar o verificador no host:

```sh
cargo build -p lyra-upgrade-verify --locked
python3 scripts/check-readonly-verification-vm.py \
  --kernel /caminho/para/vmlinuz \
  --binary target/debug/lyra-upgrade-verify \
  --baseline-binary /caminho/para/verificador-bf4093c \
  --log /tmp/lyra-readonly-verification.log
```

O script constrói dois RPMs sem scriptlets: consumidor e provedor. Instala-os
somente em uma VM QEMU/systemd, sem NIC, discos ou contas do host. Usa RPM,
zypper, solver, repositório local e unit/verificador reais. O repositório de
fixtures sem assinatura tem checagem GPG desabilitada **apenas dentro da VM**.
Sistema de arquivos/Btrfs, Snapper, SecureBoot e conteúdo GRUB são probes de
fixture; esse ensaio não substitui ISO, rollback ou firmware.

Primeiro coleta XML de três situações. O saudável retorna zero com nenhuma
mudança; o consumidor sem provedor disponível no sistema, mas presente no
repositório, também retorna zero e propõe instalar o provedor. Sem provedor
nem no repositório, o solver propõe desinstalar o consumidor ou ignorar a
dependência e retorna 4 ao cancelar em modo não interativo.

Depois reproduz o baseline bf4093c: o verificador instala o provedor fora do
plano e marca Completed. O candidato, chamado pela unit real, deve:

- Concluir o sistema saudável com Completed/Passed e sequência aplicada.
- Marcar ambos os sistemas quebrados como NeedsRecovery/Failed, sem sequência
  aplicada e com `POST_BOOT_DEPENDENCIES_FAILED`.
- Preservar byte a byte os arquivos do rpmdb (exceto o arquivo de lock), o
  inventário de pacotes e os payloads das fixtures nos três casos.

## Qualificação de 09/09/2026

Passaram a coleta inicial e o ensaio completo com kernel
`6.12.0-160100.4-default`. O baseline confirmou a instalação indevida; o
candidato preservou banco RPM, inventário e payloads nos três cenários.
As saídas XML reais foram incorporadas aos testes em
`verifier/tests/fixtures/verify-*.xml`. Também passaram 92 testes Rust,
28 Python, fmt e Clippy, incluindo propostas não vazias com saída zero,
remoção/cancelamento, XML malformado, ações desconhecidas, campos inválidos,
saída excessiva e argumentos obrigatórios de simulação.

Esta correção atende ao bloqueio técnico #12 na branch qualificada. A integração
dos PRs e qualificação dos pacotes para publicação permanecem etapas separadas.
Nenhum verificador, zypper verify ou reparo foi executado no host.
