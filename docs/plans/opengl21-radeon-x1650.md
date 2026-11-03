# План: запуск Horizon на Radeon X1650 через OpenGL 2.x

Статус: черновик плана. Код не менялся. Решения по фазам 1–6 принимаются после фазы 0 (проверка на реальной карте).

## 1. Цель и критерии приёмки

Цель: Horizon запускается и играет на Radeon X1650 (чип R500, 2006 г., DirectX 9 / Shader Model 3.0) с драйвером OpenGL 2.x.

Предварительные критерии (уточнить после фазы 0):

- Игра открывается, мир загружается, видны ландшафт, небо, мобы, HUD и меню. Без падений в обычной игре.
- Разрешение 800×600 (или 1024×768), минимальные графические настройки, дальность прорисовки 4–6 чанков. Ориентир: не ниже 20 FPS в открытом поле на целевой машине. Числа предварительные, их нужно измерить.
- Картинка близка к wgpu-версии по геометрии, цветам и освещению. Не поддерживаются: шейдерные паки, HDR, TAA, генерация кадров.

## 2. Что известно

### 2.1. Железо

- X1650 относится к R500 (RV530/RV535): 12 пиксельных и 5 вершинных блоков, шина памяти 128 бит, 128–512 МБ памяти в зависимости от варианта платы. Источники: [videocardz](https://videocardz.net/ati-radeon-x1650-pro), [gpuzoo](https://www.gpuzoo.com/GPU-ATI/Radeon_X1650_Pro.html).
- У X1000 нет Vertex Texture Fetch: вершинный шейдер не может читать текстуры. Источники: [Wikipedia, список GPU AMD](https://en.wikipedia.org/wiki/List_of_AMD_graphics_processing_units), [Beyond3D](https://forum.beyond3d.com/threads/report-says-that-x1000-series-not-fully-sm3-0.22712/page-3).
- DirectX 9.0c, Shader Model 3.0. Vulkan-драйверов для R500 нет, DirectX 12 нет.
- Отчёт драйвера Catalyst 9.3 (rev 10.2, 8.593) для X1600/X1650 ([vgamuseum](https://vgamuseum.info/images/doc/ati/x1650.txt)):
  - OpenGL 2.1 (отчёт), GLSL 1.20, 4 draw buffers, FBO (`GL_ARB_framebuffer_object`, `GL_EXT_framebuffer_object`), NPOT-текстуры, `GL_ARB_texture_float`, VBO, `GL_ARB_depth_texture`, `GL_ARB_shadow`, `GL_ARB_occlusion_query`, анизотропия 16×, `GL_EXT_texture_compression_s3tc`, `GL_EXT_multi_draw_arrays`, максимальная текстура 4096.
  - В списке расширений нет: `GL_EXT_texture_array`, `GL_ARB_draw_instanced`, `GL_ARB_instanced_arrays`, uniform buffers, sampler objects, texture storage, multisample-текстуры, compute, timer query. Считаем их отсутствующими, пока не проверено на карте.
  - Максимум native ALU-инструкций фрагментного шейдера: 512. Uniform-компоненты: 512 для вершинного и фрагментного шейдера.
- Linux: драйвер r300g (Mesa) даёт OpenGL 2.1 для R300–R500 ([Phoronix](https://www.phoronix.com/news/ODQ4OQ)). Набор расширений может отличаться от Catalyst.
- Windows: AMD последним официально поддерживавшим X1000 назвала Catalyst 10.2 (2010), для Windows 8 и новее официальной поддержки нет ([Wikipedia](https://en.wikipedia.org/wiki/Radeon_X1000_series)). Если установлен только драйвер Microsoft, OpenGL скорее всего программный и ниже 2.0. Тогда запуск невозможен. Это проверяется фазой 0.

### 2.2. Репозиторий

- Рендер построен на wgpu 30 (`crates/horizon-client-render/Cargo.toml`: features vulkan, metal, dx12, gles).
- Выбор API: `GraphicsApi { Auto, Vulkan, Dx12, OpenGl }` (`crates/horizon-client-render/src/graphics.rs:178`). Auto на Linux идёт Vulkan, затем GL (`gfx.rs:1644-1648`).
- Существующий путь OpenGl — это wgpu GLES. Он требует GL 3.3 и выше (`third_party/wgpu-hal/src/gles/adapter.rs:266-268`). На X1650 он не стартует. Это отказ, а не «почти работает».
- Компилятор шейдеров naga (`third_party/naga/src/back/glsl/mod.rs`) выпускает GLSL только версий 330+ (desktop) и ES 300/310. WGSL → GLSL 1.10/1.20 через него не получится.
- Объём: 68 файлов и около 56,7 тыс. строк Rust в `horizon-client-render` напрямую используют wgpu.
- WGSL: около 5,5 тыс. строк всего. Ядро игры (terrain, entity, hud, screen, border, stars, panorama, menu_blur, static_entity, post_chain, world_post) около 2,1 тыс. строк. Остальное (frame_gen около 1,4 тыс., visbuffer, gpu_culling, oit, hdr, shader-api) в MVP не нужно.
- `terrain.wgsl`:
  - вершины читаются из storage-буфера (`arena_page`, строки 407–420) по индексу `vertex_index`, то есть вытягиванием вершин (vertex pulling);
  - 8 массивов текстур `texture_2d_array` (строки 46–57);
  - упаковка u32-битами (`packed_corner`).
  - В GLSL 1.20 нет целых чисел и битовых операций, поэтому такой код не переносится как есть.
- `gfx/flat.rs` эмулирует indirect multi-draw через compute-шейдер. Для GL 2.x это не подходит: compute нет.
- Сборка идёт без `target-cpu`, то есть на базовом x86-64 (SSE2). В `Cargo.toml` есть исправление для старых CPU без POPCNT (Core 2, #7264). Для машин того же времени это важно.

## 3. Вывод

Включить OpenGL 2.1 в текущем рендерере нельзя. Нужен второй рендерер (назовём его GL2), который:

- не использует wgpu и naga;
- переиспользует CPU-часть: мир, освещение, мешинг (`mesher.rs`), стриминг, раскладку HUD, меню, настройки, ввод, игровой цикл;
- переписывает GPU-часть: шейдеры (вручную в GLSL 1.10/1.20 по образцу WGSL), буферы, текстуры и проходы рендера.

Замены возможностей:

| Что используется сейчас | Где | Замена в GL2 |
|---|---|---|
| Вытягивание вершин из storage-буфера, упаковка u32 | `terrain.wgsl`, `mesher/packed.rs`, `visbuffer.wgsl` | Обычный VBO с атрибутами (позиция, UV, слой, свет, цвет). Мешер выдаёт float-вершины без битовых упаковок |
| Indirect multi-draw | `gfx/cull.rs`, `gpu_culling.rs`, `displace.rs`, `gfx.rs` | CPU-отсечение секций и по одному draw на секцию и страницу атласа |
| Compute-шейдеры (12 модулей: animate, entity_skin, flatten, gpu_culling, frame_gen, weather, pack_post и др.) | `gfx/*.rs` | Работа на CPU (анимация текстур, скиннинг, погода) или отключение |
| `texture_2d_array` (8 массивов спрайтов) | `terrain.wgsl`, `hud.wgsl`, `hdr_stats.wgsl` | Несколько 2D-атласов (NPOT, до 4096). Draw-вызовы делятся по страницам |
| Instancing (14 мест с `VertexStepMode::Instance`) | `entity.wgsl`, `static_entity.wgsl`, рендер сущностей | Геометрия собирается на CPU и отправляется батчами |
| Битовые и целые операции в шейдерах | `terrain.wgsl`, `visbuffer.wgsl`, `oit*` | Только float-математика; данные заранее распаковываются на CPU |
| `textureLoad` и `texture_depth` | `world_post`, `cull`, `pack_post`, `hdr` | `texture2D` с `ARB_depth_texture`; выборку по центру текселя |
| Больше 4 MRT, float-блендинг (`FLOAT32_BLENDABLE`), HDR-форматы | `oit.rs`, `hdr.rs`, `terrain.wgsl` | Не делаем. Одна 8-битная LDR-цель, прозрачность сортируется на CPU |
| Timestamp-запросы | `timing.rs` | CPU-замеры (расширения timer query на X1650 нет) |
| Текстурные атомики и storage-текстуры | `pack_images.rs` | Не делаем |
| Vertex texture fetch | — | Не нужен; смещения и displacement считаются на CPU |

## 4. Варианты

**A. Доработать wgpu-hal и naga под GL 2.1.** Отклоняем. naga не выпускает GLSL 1.20. wgpu-hal рассчитан на VAO, UBO, samplers, SSBO и instancing. Объём работы больше, чем у самого GL2-рендерера, и вести форк придётся постоянно.

**B. Отдельный GL2-рендерер.** Рекомендуется. Описан в разделе 5.

**C. D3D9-рендерер для Windows.** Запасной вариант. X1650 поддерживает SM 3.0, а без VTF это в целом подходит. Но это вторая графическая API, только для Windows, и затраты сравнимы с B. Используем, если драйвер OpenGL на целевой ОС не даёт 2.0.

**D. Запуск через переопределение версии в Mesa (`MESA_GL_VERSION_OVERRIDE`) или программный рендер.** Отклоняем. Шейдеры GL 3.3 требуют возможностей, которых у R500 нет, а llvmpipe на старом CPU слишком медленный.

## 5. Фазы

Оценки сроков грубые, для одного опытного разработчика на полную занятость. Каждая фаза заканчивается проверкой (gate).

### Фаза 0. Разведка железа (1–2 недели)

- Утилита `gl2-probe`: создаёт контекст OpenGL 2.1 (совместимый профиль) и выводит в JSON: GL_VERSION, GLSL, все расширения, MAX_TEXTURE_SIZE, MAX_DRAW_BUFFERS, MAX_VERTEX_TEXTURE_IMAGE_UNITS, MAX_VARYING_FLOATS, а также минимальный тест: треугольник с текстурой, FBO, NPOT, mipmap через `SGIS_generate_mipmap`.
- Запуск на реальной X1650: Windows с драйвером Catalyst 10.2 и с драйвером Microsoft; Linux с r300g.
- Замер: draw-вызовы в секунду, пропускная способность вершин, скорость загрузки текстур.

**Gate G0:** есть GL 2.0, GLSL 1.10, FBO, VBO, NPOT на целевой ОС. Если нет, основным становится вариант C (Windows) или проект останавливается.

### Фаза 1. Каркас (2–3 недели)

- Новый крейт `horizon-client-gl2` без зависимости от wgpu и naga. Окно и контекст через winit (уже в проекте) и glutin.
- Выбор бэкенда: новое значение в `GraphicsApi` или автоопределение. Если GL < 3.3 и нет Vulkan, включается GL2. Если GL < 2.0, показывается понятная ошибка. Текст ошибки и новые пункты меню идут через ключи перевода (AGENTS.md).
- Определить, какая часть публичного API `Renderer` нужна игре (`game.rs` и `render_tick.rs` вызывают подмножество из ~60 методов), и вынести её в общий интерфейс или enum `Gfx { Wgpu, Gl2 }`.

**Gate G1:** окно игры очищается, выводится текстурный квадрат с атласом блоков.

### Фаза 2. Мир (4–6 недель)

- Загрузка атласов в 2D-страницы; mipmap через `SGIS_generate_mipmap`.
- Секции: CPU-мешер выдаёт float-вершины, VBO на секцию и страницу атласа, отсечение секций на CPU.
- Шейдер ландшафта в GLSL 1.10/1.20: lightmap, туман, альфа-отсечение (cutout), эмиссия. Прозрачные секции сортируются на CPU.
- Небо: градиент, солнце и луна, звёзды, восход (`terrain.wgsl`, `stars.wgsl`, `panorama.wgsl`).

**Gate G2:** деревня при дальности 4 чанков видна и совпадает с wgpu-версией в пределах порога разницы (см. раздел 7).

### Фаза 3. Сущности, интерфейс, эффекты (3–5 недель)

- Сущности: модели собираются на CPU и отправляются батчами, без instancing. Блочные сущности и предметы в руке.
- HUD, экраны и текст (`hud.wgsl`, `screen.wgsl`, `border.wgsl`). Размытие меню (`menu_blur.wgsl`) можно отложить.
- Частицы: CPU-квады, батчи по текстуре.
- Погода и дождь на CPU.

**Gate G3:** можно играть час без падений в обычном мире, включая бой, инвентарь и меню.

### Фаза 4. Настройки и урезанный режим (1–2 недели)

- Неподдерживаемые настройки показываются серыми, с тем же механизмом «сейчас недоступно», что описан в `docs/graphics.md`.
- Отключаются: шейдерные паки, HDR, генерация кадров, TAA и апскейл, GPU culling, visbuffer, OIT, асинхронное отсечение сущностей, GPU-таймеры.
- Новые строки интерфейса идут через ключи перевода.

### Фаза 5. Производительность (2–4 недели, итерации)

- Бюджет CPU, а не только GPU: на машине этой эпохи узкое место чаще в CPU и в памяти. Сокращение draw-вызовов, батчинг, минимизация смен состояния.
- Потоковая загрузка VBO с ограничением на кадр (VRAM 128–256 МБ).
- Сжатие S3TC для атласа (если расширение есть). Меняет картинку, поэтому только по решению.
- Снять замеры по тем же сценам, что в `docs/graphics.md`: village, forest, ocean, mountain.

### Фаза 6. Тесты и выпуск (1–2 недели)

- Бета-режим по выбору игрока, затем автоматический выбор для GL < 3.3 после проверки на железе.
- Тест-матрица на реальных картах: Windows (драйвер Catalyst или Microsoft) и Linux (r300g), разрешения, дальность, сцены, день и ночь, дождь.

## 6. Что не входит в GL2-режим

- Шейдерные паки (`shaders/`, OptiFine/Iris-подобный формат): GLSL 1.50+ и naga, не переносится.
- HDR, TAA, апскейл, генерация кадров, оптический поток.
- Визуальные улучшения, которые держатся на compute или float-целях.
- Vulkan и DirectX 12 на этой карте недоступны.

## 7. Тестирование

- **Фаза 0 на реальной карте обязательна.** Без неё план не утверждаем.
- **Сравнение кадров.** Есть `render-capture` и `scene_replay` (`docs/graphics.md`), но replay работает через wgpu `Renderer`. Для GL2 нужен отдельный драйвер воспроизведения той же сцены. Сравниваем GL2 и wgpu по порогу разницы (PSNR или поточечно), а не по точному совпадению.
- **CPU-паритет.** Мешер, освещение, стриминг и parity-харнесы остаются без изменений: они описывают спецификацию (vanilla) и относятся к CPU-части.
- **CI без X1650.** Mesa llvmpipe с `MESA_GL_VERSION_OVERRIDE=2.1` и `MESA_GLSL_VERSION_OVERRIDE=120` проверит компиляцию GLSL 1.20 и отказ от запрещённых фич. Это не проверяет лимиты железа, и поведение нужно подтвердить.
- **Производительность.** `tools/graphics_bench.py` рассчитан на wgpu и RTX 4080. Для X1650 нужен свой замер: FPS, CPU-мс на кадр, draw-вызовов за кадр, использование VRAM.

## 8. Риски

1. **Драйвер.** Главный риск. Windows без официального драйвера, r300g может быть медленным или с ошибками. Фаза 0 снимает неопределённость.
2. **CPU и память старой машины** важнее GPU. Пиксельная скорость карты (около 2,4 Гпикс/с по спецификации) не узкое место. Узкие места: вершины, draw-вызовы, CPU.
3. **VRAM 128–256 МБ:** атлас и меши секций. Может понадобиться S3TC или меньшая дальность.
4. **Два рендерера:** дублирование багов, расхождение визуала. Нужна документация отличий.
5. **Лимиты GLSL 1.10/1.20:** 512 ALU-инструкций на фрагмент и 512 uniform-компонентов. Тяжёлые шейдеры придётся делить на проходы.
6. **Нет целых чисел и битовых операций** в шейдерах. Данные распаковываются на CPU.
7. **Нет CI-железа.** Регрессии на X1650 будут ловиться только вручную.

## 9. Что нужно уточнить

- Какая ОС стоит на машине с X1650 (Windows XP/7/10, Linux и дистрибутив)?
- Модель карты (X1650 Pro или XT, PCIe или AGP, объём памяти), CPU и объём RAM.
- Какие цифры считать успехом: разрешение, дальность, FPS.
- Готовы ли принять урезанный визуал (без шейдерных паков, HDR и т. д.)?
- Есть ли физический доступ к карте или тестер для фазы 0?

## 10. Оценка объёма

Грубо: 3–6 месяцев одного опытного разработчика на полную занятость. Фаза 0 уточнит оценку. Если драйвер плохой, стоимость растёт, и вариант C становится основным.

## Источники

- [vgamuseum: Radeon X1600/X1650, отчёт драйвера Catalyst 9.3](https://vgamuseum.info/images/doc/ati/x1650.txt)
- [videocardz: ATI Radeon X1650 Pro](https://videocardz.net/ati-radeon-x1650-pro)
- [gpuzoo: ATI Radeon X1650 Pro](https://www.gpuzoo.com/GPU-ATI/Radeon_X1650_Pro.html)
- [Wikipedia: Radeon X1000 series](https://en.wikipedia.org/wiki/Radeon_X1000_series)
- [Wikipedia: List of AMD graphics processing units](https://en.wikipedia.org/wiki/List_of_AMD_graphics_processing_units)
- [Beyond3D: Report says that X1000 series not fully SM3.0?](https://forum.beyond3d.com/threads/report-says-that-x1000-series-not-fully-sm3-0.22712/page-3)
- [Phoronix: ATI R600 Mesa classic driver can do OpenGL 2.1 (о R300–R500 через r300g)](https://www.phoronix.com/news/ODQ4OQ)
- Репозиторий: `crates/horizon-client-render/src/graphics.rs`, `gfx.rs`, `terrain.wgsl`, `gfx/flat.rs`, `third_party/wgpu-hal/src/gles/adapter.rs`, `third_party/naga/src/back/glsl/mod.rs`, `docs/graphics.md`.
