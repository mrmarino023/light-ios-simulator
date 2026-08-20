# Homebrew (optional)

MIT open-source install via Homebrew HEAD (from a clone):

```bash
git clone https://github.com/mrmarino023/light-ios-simulator.git
cd light-ios-simulator
brew install --HEAD ./Formula/ligh.rb
```

Then:

```bash
ligh doctor
ligh daemon start
ligh up
```

Prefer `./scripts/install.sh` if you are already hacking on the repo — same MIT binaries, no tap required.
