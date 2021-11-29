# rbrb

A library for building RoBust RollBack-based networked games.

`rbrb` is heavily inspired by [GGPO](https://www.ggpo.net/) and
[GGRS](https://github.com/gschup/ggrs), but aims to be more reliable and capable.

## Assumptions

This library assumes your game is a deterministic `Fn(&State, Set<Input>) -> State`.
We (will) have an additional testing mode that will spend extra cycles on checking that the
state is consitent between players and deterministic on the same logical update.

## Roadmap

### Core Functionality

- [x] Multi-party sync
- [ ] Consistent disconnection
- [ ] Reconnect disconnected player

### Robustness

- [ ] Determinism checks
- [ ] Checksum propagation
- [ ] Debugging failed checks
- [x] Fake a bad network
- [x] Confirmation state

### Features

- [ ] In-game replays
- [ ] Out of game replays
  - [ ] Headless
- [ ] Spectators
  - [ ] Drop in/out
- [ ] Multiple local players

### Performance

- [x] Sparse inputs
- [ ] Input delta encoding
- [ ] Hub and spoke network

License: MIT
