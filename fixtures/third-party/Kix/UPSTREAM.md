Kix (byKosta/Kix-app) is vendored for LIGH validation.
Upstream: https://github.com/byKosta/Kix-app
Commit: 0bc85035cc779abc4fd6ff05065034932bb4c744
Upstream has no LICENSE file in-tree at vendoring time — treat as sample/demo source, not redistributable product IP.

Local adaptations (not score-tuning):
- IPHONEOS_DEPLOYMENT_TARGET lowered 26.1 → 17.0 so current Xcode can build
- Login email field rewritten as a plain TextField with accessibilityIdentifier on the
  control itself (upstream customField container was AX-hittable but not typeable)

Do not add per-app Autopilot branches. Bug patches live under scenarios/ and frozen tasks only.
