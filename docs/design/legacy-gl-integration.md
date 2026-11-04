# Legacy GL: регламент интеграции и CI01

Статус: **действующие правила процесса**, 2026-11-04. Область: работы по
[плану запуска](legacy-gl-launch-plan.md), не весь проект Horizon.
Назначение владельца подтверждено пользователем в этой сессии.

## 1. Главный интегратор и полномочия

**Главный интегратор — Arena.ai Agent, ведущий эту инициативу от имени `lukintrees`
в ветке `arena/a7e96628-horizon`.** Именно этот интегратор:

- утверждает и изменяет общий renderer facade, CPU frame/resource/platform contracts;
- сам вносит изменения в центральные manifests/lockfiles и feature wiring;
- владеет интеграционными правками Game startup/loop и общим GL dispatcher;
- согласует границы PR, зависимости, pass slots, contract versions и порядок интеграции;
- проверяет кандидатов, интегрирует согласованные изменения и обновляет проверенный baseline.

Другие исполнители работают в назначенных modules/adapters/tests. **Не менять общий facade,
contracts, центральные manifests/lockfiles самостоятельно, даже «на одну строку» или для
устранения compile error.** Не копировать и не обходить контракт вторым несовместимым API.
Нужное изменение оформляется запросом интегратору; код в общей зоне пишет интегратор.

Это роль процесса, а не новый GitHub-аккаунт или отмена полномочий пользователя/maintainers.
Существующие CODEOWNERS и правила репозитория сохраняются. Интегратор не подменяет необходимое
человеческое ревью и не может оформить GitHub approval собственного PR от другого имени.
Назначение другого интегратора требует явного решения пользователя и обновления этого документа.
Открытие draft PR не является разрешением на merge в main или release.

## 2. Зоны владения

Все перечисления относятся к legacy-инициативе. Уже существующие более строгие правила
CODEOWNERS/безопасности применяются дополнительно.

| Зона | Кто пишет | Что допускается остальным |
|---|---|---|
| Общий facade и CPU Frame/resource/platform contracts, re-exports | **Только главный интегратор** | Предложения интерфейса и consumer tests в своём модуле после согласования |
| Планируемые `renderer_facade`, `render_contract`, `platform_contract` и их public exports | **Только главный интегратор** | Не создавать эти файлы независимо; имена/пути фиксирует C01 |
| `crates/horizon-render-api/src/lib.rs`, общие `entity.rs`, `scene.rs`, `paged_gui.rs` contracts | **Только главный интегратор** | Запрос policy/API change + acceptance fixture, не самостоятельный contract patch |
| `crates/horizon-client/src/game.rs`, `entry.rs`, общий `game/event_loop.rs`, startup/factory wiring | **Только главный интегратор** для этой инициативы | Отдельные согласованные adapter modules; shared wiring выполняет интегратор |
| `crates/horizon-client-render/src/lib.rs`, `gfx.rs`, `frame/**`, `legacy_gfx.rs`, `legacy_gfx/frame.rs` | **Только главный интегратор** для общих API/dispatch/wiring | Изолированные pass/preparation modules по согласованным hook/contract |
| `crates/horizon-legacy-render/src/renderer.rs`, общие `terrain.vert`/`terrain.frag`, native exports | **Только главный интегратор** для dispatcher/shader wiring | Изолированная platform/math/preparation работа по выделенному scope |
| Root/workspace и затрагиваемые legacy-работами crate `Cargo.toml`, `Cargo.lock`, включая новые crates | **Только главный интегратор** | Dependency/feature request с обоснованием, целями сборки и MSRV/Win7 рисками |
| CI workflows/policy scripts, этот регламент и интеграционные указания в AGENTS.md | **Только главный интегратор**, с действующим human/code-owner review | CI01 design/tests и предложения; не ослаблять gates своим PR |
| Отдельные pass modules, CPU preparations, adapters, собственные fixtures/docs | Назначенный исполнитель | Работать в заранее согласованном списке файлов и public API version |

C01 должен опубликовать точные пути новых facade/contract файлов **до** первой кодовой волны.
Этот список дополняется при появлении новых общих точек. D01 не даёт отдельному исполнителю права
редактировать manifests: dependency owner готовит аудит/план, интегратор применяет кодовые изменения.
Новые manifest/lockfile изменения не прячутся в generated files или в «formatting only» PR.

## 3. Base, draft и единица интеграции

- Текущий **integration draft**: head `arena/a7e96628-horizon`, base `main`.
  Он содержит накопленный проверенный GL2 foundation, а не только документы процесса.
  Это большая интеграционная база; его нельзя выдавать за маленький PR с одним новым эффектом
  или за готовую playable/Win7 release.
- Пока foundation/contracts не вошли в main, task PR опирается на **integration branch**, а не
  предполагает наличие этих APIs в main. Base будущего task PR согласуется до начала работы.
  Отдельные исполнители создают свои рабочие ветки; текущая интеграционная сессия остаётся на своей ветке.
- После принятия foundation в main интегратор объявляет новый base/contract version и порядок переноса
  оставшихся task PR. Не менять base всем одновременно без согласования.
- Не заливать несколько независимых задач одним commit/PR. Ориентир — небольшие, отдельно проверяемые
  изменения; большие work items из плана делятся на a/b/c.
- Любой task PR начинается как draft с `Task-ID`, `Depends-on`, scope и тестами. Зависимость считается
  доступной после интеграции/объявленного baseline, а не только по наличию открытого PR.
- Не переписывать историю после интеграторских fixups: merge актуального base, без force-push/rebase,
  в соответствии с CONTRIBUTING.md. Не cherry-pick одну реализацию в несколько конкурирующих веток.

## 4. Протокол общей правки

1. Исполнитель открывает запрос в своём PR: какой contract/manifest/wiring change нужен, почему нельзя
   сделать в своём модуле, какие consumers/targets затронуты и какой тест это подтверждает.
2. Интегратор принимает или корректирует запрос и назначает contract version/baseline.
3. **Интегратор сам пишет общую правку** отдельным commit/PR; dependency changes и lockfile обновляются
   воспроизводимо, без постороннего upgrade. Менять правила CI под падающий тест запрещено.
4. Исполнитель обновляет task branch от объявленного base и завершает только свой adapter/module.
5. Если центральные файлы всё же попали в task diff, PR возвращается в draft: правки убираются из
   task PR, нужная часть отдельно реализуется интегратором. Один approval не превращает чужой
   central patch в разрешённую форму процесса.

Для обсуждений и GitHub-комментариев агенты соблюдают attribution из AGENTS.md.
Задачи/claims следуют CONTRIBUTING.md; не открывать tracker и не claim весь проект как одну задачу.
При создании этого регламента GitHub вернул, что issues в `lukintrees/horizon` отключены:
tracking issue и `/claim` не созданы. До включения issues регистрация ограниченного scope/Task-ID/owner
ведётся в draft PR, без заявления о выполненном bot claim. Если issues включат, вернуться к обычному
claim-процессу репозитория.

## 5. Цикл кандидата и порядок merge

```text
scope/owner/dependencies → draft → реализация в своём модуле → актуальный base
    → CI01 checks + evidence → review интегратора/нужного CODEOWNER
    → ready candidate → последовательная интеграция → aggregate CI на новом head
    → объявленный baseline для следующей волны
```

**До ready:** объявленный scope соблюдён, центральные файлы не изменены, зависимости доступны,
modern путь не сломан, required checks успешны именно для текущего кандидата, ограничения указаны.
Skipped/cancelled/missing check не считать доказательством выполнения теста.

**После каждого task merge:** integration head снова проходит aggregate CI; пока он красный,
следующий независимый task не интегрируется. Исправить причину или отдельно согласовать revert,
не накапливать несколько возможных причин в общей ветке. Никакого auto-merge по label.

Вариант merge (обычный merge либо squash) согласуется интегратором с зависимостями PR; точный SHA
результата фиксируется. Не менять global branch protection или release automation в рамках обычной
render-задачи. Main integration draft остаётся draft до отдельного решения о готовности foundation.

## 6. CI01 — PR/aggregate CI для параллельной разработки

**ID: CI01, P0. Статус: CI01a/b/c реализованы первым кодовым слоем; CI01d ещё не реализован.**
C01 v1 operations/paths и ограничения зафиксированы в [legacy-gl-c01.md](legacy-gl-c01.md).

Текущая реализация:
- `.github/workflows/legacy-gl-branch.yml`: один workflow, поддерживающий push, scoped pull_request,
  workflow_dispatch и workflow_call; policy tests, default-modern check и настоящие Game loop tests,
  CPU/game GPU fixtures, independent GL2 и Windows-x86; общий `legacy/required` fail-closed.
- `.github/legacy-gl-policy.json` + `tools/ci/legacy_scope.py`: protected paths из доверенного base,
  role по точным repo/base/head, task metadata/version, обе стороны rename/delete/new manifest.
- `.github/workflows/legacy-gl-scope.yml`: metadata-only pull_request_target control plane,
  загружает **только base**, fetch PR objects без checkout/exec и публикует `legacy/scope` head status.
  Это единственный новый job с `statuses: write`; PR/build jobs остаются read-only.
- `tools/ci/legacy_required.py`: failures/skips/cancellations/missing jobs и актуальность PR base/head/merge
  через API в конце прогона. После последующего движения base требуется новый кандидат/прогон;
  автоматический запрет merge устаревшего PR требует отдельно настроенного strict branch protection.

**Bootstrap:** trusted-target workflow должен уже находиться в base. Для task PR в integration branch
он входит в текущую базу. У самого integration draft base=main пока не содержит нового gate; нельзя
утверждать, что main-side trusted gate уже установлен или новый workflow сам себя защитил в этом PR.
Его bootstrap проверяется человеком; repository rulesets/CODEOWNERS не изменены. Реальный контрольный
прогон trusted-target на task PR подтверждается при появлении первого такого PR, без открытия
конкурирующей рабочей ветки самим интегратором.
Владелец политики и wiring: главный интегратор. Реализацию изолированных тестовых помощников
можно делегировать по §2. CI01 **не** является C01: C01 фиксирует API/ownership, CI01 — как проверять
PR и общий head. Его design можно вести сразу; основную кодовую волну не объявлять merge-ready до
работающего CI01. Аудиты O01/M01/A01 и подготовку fixtures можно начинать раньше.

Прежний `.github/workflows/legacy-gl-branch.yml` проверял только собственные push integration branch.
Теперь entry points расширены, а jobs переиспользуются вместо копирования в каждый task workflow. Его зелёный старый head нельзя использовать
вместо проверки новой комбинации base + task.

### 6.1. Пакеты реализации CI01

| Пакет | Содержание | Критерий приёмки |
|---|---|---|
| **CI01a: entry points / isolation** | PR checks для согласованных task bases и integration draft; aggregate push check; переиспользуемые jobs вместо копий | Task и integration candidate действительно запускают одну версию checks; актуальный base/head/merge SHA записан; unrelated PR не получают скрытую legacy policy |
| **CI01b: scope / ownership** | Проверка changed paths, contract/version metadata, Depends-on и запрещённых task central paths | Отрицательные тесты: изменённый facade/manifest/lockfile/workflow/new manifest/rename-to-protected path блокируют task PR; текст PR/label не даёт обход |
| **CI01c: build / runtime matrix** | Раздельные modern, legacy CPU/native/GPU, Windows-x86 jobs, общий итоговый check | Required job не пропускается path filter'ом; failures/cancellations/missing results не становятся green summary; fixtures и reports публикуются |
| **CI01d: actual launch gates** | Подключить T01/T02, позже D01/O01/O02/Q01 evidence | Настоящий startup/input/save E2E отличим от renderer tests; Win7 hardware остаётся отдельным ручным evidence, а не фиктивным hosted-CI success |

### 6.2. Предлагаемые стабильные имена checks

Стабильные имена ниже используются/уточняются в новой матрице; **required branch protection ещё не настроен**.
`legacy/candidate-scope` — read-only candidate check, `legacy/scope` — отдельный trusted head status.
CPU и actual-game GPU fixtures сейчас объединены в `legacy/cpu-game`, policy fixtures — `legacy/policy-tests`.

| Check | Когда / что проверяет |
|---|---|
| `legacy/scope` | Каждый task PR: scope/ownership, contracts, зависимости; отдельная доверенная роль integration draft |
| `legacy/modern` | Затронутый modern build/tests; отсутствие регрессии текущего wgpu пути |
| `legacy/cpu` | CPU contracts/bridges/preflight и тесты новых модулей |
| `legacy/gl2` | Independent GL2 readbacks + настоящие Frame GPU fixtures; текущий baseline 3×26 и 15 fixtures, с пополнением |
| `legacy/windows-x86` | x86 compile/link/runtime CPU contracts на hosted Windows; label отчёта прямо говорит «не Win7 qualification» |
| `legacy/launch` | После готовности T01/T02: настоящий Game scenario; до этого отдельное незакрытое evidence, не фиктивный passed test |
| `legacy/required` | Проверяет полный ожидаемый набор и результаты; не просто job, который запускается после любого завершения |

Документальные PR могут иметь согласованную лёгкую матрицу. Scope check не пропускается, а итоговый
check объясняет выбранный набор. Path filtering не должно оставлять required check навечно pending или
выдавать старую зелёную проверку за запуск нового кода. Exact modern/build commands фиксируются в CI01
после C01/R01/D01; до legacy-only feature graph нельзя публиковать несуществующие команды как готовые.

### 6.3. Безопасность и доверие gate

- Read-only permissions, без secrets/release credentials в untrusted PR jobs.
  Не выполнять PR-код с привилегиями через `pull_request_target`.
- Ownership policy/checker берутся из доверенного base; изменение самого checker в task diff не может
  отменить правило для этого же PR. Обрабатывать удаления/добавления/renames и base updates.
- Integration exception не определяется одним `Role: integrator` в PR body, label или похожим названием
  ветки. Нужны явно управляемые trusted repo/base/head rules и review; forks не могут объявить себя integration.
- Обычный GitHub author/commit email не удостоверяет, какой AI-agent писал код: агенты могут работать
  под одним аккаунтом. CI не способен сам доказать требование «правку написал именно этот агент».
  Это обеспечивается запретом central task diffs, процессом запросов и ручной интеграцией.
- В required-check policy не выдавать hardware Win7/X1650 gate за выполненный на Windows-latest/Wine.
- Настройка branch protection/rulesets — отдельный согласуемый admin шаг после появления стабильных checks;
  этот документ и draft PR не меняют repository settings.

### 6.4. Матрица отрицательных тестов gate

1. Task меняет root/crate manifest, lockfile или создаёт новый manifest — reject.
2. Task меняет facade/re-export/shared Frame/dispatcher/CI policy — reject.
3. Task меняет policy, а затем сам себе разрешает центральные файлы — reject.
4. Rename/delete центрального файла, fork с похожим head name или self-assigned integration label — reject.
5. Base обновился, PR merge candidate уже другой — старые successful checks не принимаются как новый result.
6. Один required job failed/cancelled/missing — общий gate не success.
7. Обычный module-only task с доступными зависимостями и полным успешным набором — pass.

## 7. Что действует уже сейчас, а что пока только план

**Уже действует:** назначение интегратора; exclusive central ownership; запросы общих правок;
scope-first draft PR; последовательная интеграция; запрет silent fallbacks и false qualification.
AGENTS.md ссылается на этот регламент для остальных исполнителей.

**Пока не реализовано:** CI01d actual Game launch gates, repository required branch protection,
main-side trusted workflow bootstrap и подтверждение trusted-target на первом task PR. CI01 не доказывает
автоматически факт merge заявленных Depends-on: доступность зависимостей проверяет интегратор.
Существующий CODEOWNERS не заменён именем AI-agent; GitHub не превращён в эксклюзивный write ACL.
Нельзя считать manual review/OS qualification заменёнными новыми jobs.

Шаблон task PR приведён в [плане](legacy-gl-launch-plan.md#8-как-не-превратить-параллельные-pr-в-merge-bottleneck).
Заявка на shared change дополнительно содержит contract/manifest diff **в описании/предложении**,
а не самостоятельное изменение защищённых файлов в task branch.


### Обязательная metadata task PR после C01 v1

```text
Task-ID: M01a
Variant: A
Contract-Version: 1
Depends-on: none (подготовительная подзадача)
Scope: docs/design/legacy-material-audit.md; собственные audit fixtures/tests
```

Значения Scope/Depends-on — декларации для review, не право менять protected files.
Любая чужая central правка блокирует task независимо от metadata/labels.
