# Install Tarn

Tarn is a single static binary. Pick one:

```bash
# Homebrew
brew install NazarKalytiuk/tarn/tarn

# Cargo
cargo install tarn

# Install script
curl -fsSL https://raw.githubusercontent.com/NazarKalytiuk/hive/main/install.sh | sh
```

Verify with:

```bash
tarn --version
```

If the binary is not on your `PATH`, set `tarn.binaryPath` in the VS Code Settings UI to the absolute path.
