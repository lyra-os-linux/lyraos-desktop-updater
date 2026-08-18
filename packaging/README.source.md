# Fonte do pacote `lyra-upgrade`

O pacote OBS é produzido a partir do diretório `upgrade/` do repositório Lyra.
O arquivo `build-source.txt` registra o commit exato, e `vendor.tar.zst` deve
ser criado por `cargo vendor` a partir do `Cargo.lock` versionado. A chave
`release-signing-key.gpg` é a forma binária, desarmorizada e reproduzível de
`docs/release-signing-key.asc`; ela não é uma chave privada.

O build deve permanecer offline. Mudanças no lockfile, nas fontes vendorizadas
ou na chave exigem nova revisão do pacote no staging antes da promoção.

Com a árvore limpa e commitada, gere todas as fontes com:

```sh
./upgrade/packaging/make-obs-sources.sh
```

O diretório `upgrade/packaging/output/` conterá `Source0` a `Source3` e um
`SHA256SUMS`. O script usa o timestamp do commit, normaliza proprietário,
ordenação e metadados dos tarballs, exige `Cargo.lock` e executa
`cargo vendor --locked`. Somente esses artefatos revisados devem ser enviados
ao staging; o OBS não acessa a rede durante o build.
