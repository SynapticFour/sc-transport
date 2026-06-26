# Releasing

1. Ensure CI is green on `main`.
2. Update `CHANGELOG.md`.
3. Tag: `git tag -a vX.Y.Z -m "vX.Y.Z"` && `git push origin vX.Y.Z`
4. Verify release: `sc-transport-offline-*.tar.gz`, `SHA256SUMS.txt`, `install.sh`, `sct.toml.example`.

Customers must set **`SCT_VERSION`** in `.env`. Synaptic-Core `VERSIONS.lock` documents compatible releases.
