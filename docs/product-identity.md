# Product identity

The user-facing product name is **PlayerVox OverCrow**.

Every application title, launcher name, HTML title, installer message, and
user-facing test or instruction must use that exact word order. The interface
must not compose `OverCrow` with a separate `by PlayerVox` byline.

Technical compatibility identifiers are not product copy and remain stable:

- desktop and AppStream ID: `com.playervox.OverCrow`
- D-Bus names and object paths under `io.github.overcrow`
- package, binary, service, configuration, and persisted key names

Tests should reject the legacy visible name `OverCrow by PlayerVox` while
allowing descriptive prose that identifies PlayerVox as the distributor.
