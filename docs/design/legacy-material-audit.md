# Legacy GL2: аудит material codes и Frame/HUD flags (M01a)

Статус: **draft, каталог в работе**. Task-ID: **M01a**, вариант A (Arena.ai Agent Mode).
Integration base: `arena/a7e96628-horizon`, baseline SHA
`453966cb6253adf848deb58165eba354693e7acc` (срез кода, на котором подтверждены
все producer/call site ниже). Правила: [legacy-gl-integration.md](legacy-gl-integration.md),
план: [legacy-gl-launch-plan.md](legacy-gl-launch-plan.md) (M01).

Назначение: для каждого реального producer'а material codes и Frame/HUD flags
зафиксировать call site, поддержку в текущем legacy-пути (converter + preflight +
dispatcher), требуемую blend/depth/sampling policy и существующий/недостающий тест;
отдельно выделить P0-блокеры первого мира (B2) и optional visuals.

Каталог подтверждён исходниками, не synthetic quads. Воспроизводимые проверки —
изолированные audit fixtures/tests на существующем API без его изменения.

<!-- Полные таблицы и P0-список добавляются в этом PR. -->
