# Update Forge-Java Submodule

Update the forge-java submodule to the latest upstream commit and verify card loading.

## Steps

### 1. Initialize submodule (if needed)

```bash
git submodule init
git submodule update
```

### 2. Pull latest from upstream

The submodule tracks the `master` branch of `git@github.com:rrnewton/forge.git`.

```bash
cd forge-java
git checkout master
git pull origin master
cd ..
```

### 3. Verify cardsfolder is populated

The `cardsfolder` symlink points to `forge-java/forge-gui/res/cardsfolder/`.
After update, it should contain ~31k+ card script files.

```bash
ls -la cardsfolder  # Should be a symlink to forge-java/forge-gui/res/cardsfolder/
ls cardsfolder/ | wc -l  # Should be ~31000+
ls cardsfolder/ | head -5  # Spot check: should show letter-named subdirectories (a, b, c...)
```

If cardsfolder is broken or empty, restore the symlink:
```bash
ln -sf forge-java/forge-gui/res/cardsfolder/ cardsfolder
```

### 4. Run card loading tests

```bash
cargo test core::card  # Card data structure tests
cargo test card_loading  # Card script parsing (if available)
```

### 5. Re-export WASM card data

If the card database changed, re-export for the web UI:
```bash
make wasm-export
```

### 6. Stage the submodule update

```bash
git add forge-java
# Don't commit yet — let the caller decide when to commit
```

## Troubleshooting

- **Submodule clone fails**: May need SSH key setup for github.com. Try `ssh -T git@github.com` to verify.
- **cardsfolder empty after update**: The symlink target `forge-java/forge-gui/res/cardsfolder/` may not exist if the submodule didn't fully clone. Re-run `git submodule update --init --recursive`.
- **NEVER run `git clean -fxd`** in this repo or the submodule — it destroys valuable configuration.
