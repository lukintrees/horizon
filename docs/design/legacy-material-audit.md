# Аудит material codes и Frame/HUD флагов legacy GL2 (M01a)

> 🤖 Статус: **в работе**, каркас. Итоговый каталог появится в следующих коммитах этого PR.

- **Task-ID:** M01a (условный ID M01 из [плана запуска](legacy-gl-launch-plan.md); работа разбита на a/b/c по [регламенту](legacy-gl-integration.md#3-base-draft-и-единица-интеграции))
- **Variant:** A — аудит каталога и воспроизводимые проверки источников, без новых passes
- **Scope:** `docs/design/legacy-material-audit.md`, изолированные audit fixtures и тесты. Central-файлы не меняются: facade/contracts, `game.rs`/`entry.rs`, `legacy_gfx.rs`, dispatcher, `Cargo.toml`/`Cargo.lock`, workflows, policy scripts.
- **Depends-on:** ничего кодового; аудит читает текущий код на baseline
- **Integration base:** `arena/a7e96628-horizon`
- **Baseline SHA:** `453966cb6253adf848deb58165eba354693e7acc` (проверено fetch'ем; на момент записи совпадает с головой базы)
- **Central files changed:** none
- **Issues:** в репозитории отключены; claim не создан

Каталог, P0-список и непроверенные ограничения будут добавлены сюда до запроса review.
