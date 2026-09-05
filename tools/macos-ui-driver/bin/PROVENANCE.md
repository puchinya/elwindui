# macos-ui-driver checked-in E2E binary provenance

- Binary: `tools/macos-ui-driver/bin/macos-ui-driver`
- Binary SHA-256: `2c42195cbceb82b1159b900813fdc50e1cd6877c0c43912503003cdc44f2e587`
- Source fingerprint: `b35e53836f876fd6ec86833d1aa03da4726c9f4cbb72ae30e45149d3bbc78506`
- Source fingerprint command:
  `(cd tools/macos-ui-driver && find Package.swift Sources -type f -print0 | sort -z | xargs -0 shasum -a 256) | shasum -a 256`
- Build command: `swift build -c debug` in `tools/macos-ui-driver`; copied from `.build/debug/macos-ui-driver`
- Swift version: `Apple Swift version 6.3.3 (swiftlang-6.3.3.1.3 clang-2100.1.1.101)`
- Target architecture: `arm64-apple-macosx26.0`
- Build macOS version: `26.6.2 (Build 25G83)`
- Built at: `2026-09-06` (checked-in binary path mtime `01:14:39 +0900`)
- Reason for this binary revision: driver-source remediation added the native `resize` command;
  the binary was rebuilt and permission-checked before this master-sync remediation, then
  preserved unchanged during the master merge.
- TCC verification:
  - Accessibility: `true` (host-context `doctor` in the delegated AppKit E2E session)
  - Screen Recording: `true` (host-context `doctor` in the delegated AppKit E2E session)

The source fingerprint is computed over `Package.swift` and every file below `Sources/`, using
the exact command above. Future driver-source changes must rebuild this binary, preserve mode
`100755`, update this file, run the freshness verifier, and rerun host-context `doctor`.
