# macos-ui-driver checked-in E2E binary provenance

- Binary: `tools/macos-ui-driver/bin/macos-ui-driver`
- Binary SHA-256: `045ba8a70661e8b48dd38936b0d8ae0977eb484fcf59fa81809e750406d84fe9`
- Source fingerprint: `21757a326094c2442f8754013e06f29f69967ae987121e40259ff5312fb12f6c`
- Source fingerprint command:
  `(cd tools/macos-ui-driver && find Package.swift Sources -type f -print0 | sort -z | xargs -0 shasum -a 256) | shasum -a 256`
- Build command: `swift build -c debug` in `tools/macos-ui-driver`; copied from `.build/debug/macos-ui-driver`
- Swift version: `Apple Swift version 6.3.3 (swiftlang-6.3.3.1.3 clang-2100.1.1.101)`
- Target architecture: `arm64-apple-macosx26.0`
- Build macOS version: `26.6.2 (Build 25G83)`
- Built at: `2026-09-12` (checked-in binary rebuilt in host context)
- Reason for this binary revision: driver-source remediation added the direct AX `set-value`
  command and exposes description/range metadata in AX observations so semantic text/value and
  slider-range assertions can be tested without relying on keyboard injection.
- TCC verification:
  - Accessibility: `true` (host-context `doctor` in the delegated AppKit E2E session)
  - Screen Recording: `true` (host-context `doctor` in the delegated AppKit E2E session)

The source fingerprint is computed over `Package.swift` and every file below `Sources/`, using
the exact command above. Future driver-source changes must rebuild this binary, preserve mode
`100755`, update this file, run the freshness verifier, and rerun host-context `doctor`.
