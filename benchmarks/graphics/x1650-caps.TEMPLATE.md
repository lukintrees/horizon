# Снятие реальных капсов X1650 (PR 1)

В песочнице Arena нет видеокарты Radeon X1650 и нет `glxinfo`, поэтому здесь лежит
**только шаблон**, а не измерение. Файл `x1650-reference-caps.json` /
`x1650-reference-caps.txt` рядом — это эталон, пересобранный из публичного дампа
Catalyst 9.3 (`https://vgamuseum.info/images/doc/ati/x1650.txt`), с явной пометкой
`measured_on: null` и `source: ... NOT measured in this sandbox`.

## На целевой машине (Linux + Mesa r300g + X11/GLX)

```sh
glxinfo -B -l > /tmp/x1650.txt
python3 tools/gl21_caps.py --from-file /tmp/x1650.txt \
    --label x1650 --output-dir benchmarks/graphics \
    --source-note 'Radeon X1650, Mesa r300g, GLX' --check
```

Ожидается `tier L1` (OpenGL 2.1 / GLSL 1.20). Если драйвер отдаёт только 2.0 /
GLSL 1.10, инструмент выберет `L0`; если GLSL-компилятор не работает — `F0`.
`--check` вернёт 1, когда не хватает `GL_ARB_vertex_buffer_object`,
`GL_ARB_multitexture`, `GL_ARB_texture_non_power_of_two` или FBO-расширения.

Для Mesa r300g повторите с другим ярлыком:

```sh
python3 tools/gl21_caps.py --label x1650-r300g --check
```

## В CI без железа

Mesa меняет **отчёт** о версии, но не реальные возможности драйвера, поэтому
одновременно нужен статический тест `tests/test_gl21_contract.py` и проверка, что
`gfx21/` не вызывает GL3+-функций (PR 9):

```sh
LIBGL_ALWAYS_SOFTWARE=1 MESA_GL_VERSION_OVERRIDE=2.1 \
MESA_GLSL_VERSION_OVERRIDE=120 MESA_EXTENSION_MAX_YEAR=2006 \
python3 tools/gl21_caps.py --label ci-gl21 --check --no-write --stdout
```

Подробнее: `docs/design/legacy-gl21.md`, §1 и §7.
