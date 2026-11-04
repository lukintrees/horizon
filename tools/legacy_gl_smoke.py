#!/usr/bin/env python3
"""Headless developer smoke test for the proposed GL 2.1 renderer (Mesa EGL).

Run with LIBGL_ALWAYS_SOFTWARE=1 MESA_GL_VERSION_OVERRIDE=2.1
MESA_GLSL_VERSION_OVERRIDE=120 python3 tools/legacy_gl_smoke.py
This is a probe, not the game's renderer or an emulation of an AMD driver.
"""
import argparse
import ctypes as C
import ctypes.util
import os
import re
import sys
from pathlib import Path

EGL_NONE = 0x3038
W = H = 32


def library(name):
    path = ctypes.util.find_library(name)
    if not path:
        raise RuntimeError(f"{name} library not found; install Mesa EGL and OpenGL libraries")
    return C.CDLL(path)


def bind(lib, name, result, *args):
    fn = getattr(lib, name)
    fn.restype, fn.argtypes = result, args
    return fn


def version_at_least(value, major, minor):
    match = re.match(r"^(\d+)\.(\d+)", value)
    return bool(match) and tuple(map(int, match.groups())) >= (major, minor)


def check_pixels(center, corner):
    # The texture is (200, 40, 20, 128), alpha-blended over opaque black.
    return (len(center) >= 3 and len(corner) >= 3
            and all(abs(int(v) - expected) <= 3 for v, expected in zip(center[:3], (100, 20, 10)))
            and all(int(v) <= 3 for v in corner[:3]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--osmesa", action="store_true", help="use OSMesa instead of surfaceless EGL")
    args = parser.parse_args()
    if os.environ.get("LIBGL_ALWAYS_SOFTWARE") != "1":
        raise RuntimeError("set LIBGL_ALWAYS_SOFTWARE=1 to require Mesa software rendering")
    if args.osmesa:
        run_osmesa()
        return
    egl, gl = library("EGL"), library("GL")
    I, U, P, F = C.c_int, C.c_uint, C.c_void_p, C.c_float
    get_proc = bind(egl, "eglGetProcAddress", P, C.c_char_p)
    get_platform_addr = get_proc(b"eglGetPlatformDisplayEXT")
    if not get_platform_addr:
        raise RuntimeError("EGL_EXT_platform_base is unavailable")
    get_platform = C.CFUNCTYPE(P, U, P, C.POINTER(I))(get_platform_addr)
    display = get_platform(0x31DD, None, None)  # EGL_PLATFORM_SURFACELESS_MESA
    initialize = bind(egl, "eglInitialize", U, P, C.POINTER(I), C.POINTER(I))
    terminate = bind(egl, "eglTerminate", U, P)
    if not display or not initialize(display, None, None):
        raise RuntimeError("could not initialize a surfaceless EGL display")
    surface = context = None
    try:
        bind_api = bind(egl, "eglBindAPI", U, U)
        choose = bind(egl, "eglChooseConfig", U, P, C.POINTER(I), C.POINTER(P), I, C.POINTER(I))
        create_surface = bind(egl, "eglCreatePbufferSurface", P, P, P, C.POINTER(I))
        create_context = bind(egl, "eglCreateContext", P, P, P, P, C.POINTER(I))
        make_current = bind(egl, "eglMakeCurrent", U, P, P, P, P)
        if not bind_api(0x30A2):  # EGL_OPENGL_API
            raise RuntimeError("EGL does not support desktop OpenGL")
        attrs = (I * 15)(0x3033, 1, 0x3040, 8, 0x3024, 8, 0x3023, 8,
                         0x3022, 8, 0x3021, 8, 0x3025, 16, EGL_NONE)
        config, count = P(), I()
        if not choose(display, attrs, C.byref(config), 1, C.byref(count)) or not count.value:
            raise RuntimeError("no EGL pbuffer OpenGL config with RGBA and depth")
        surface = create_surface(display, config, (I * 5)(0x3057, W, 0x3056, H, EGL_NONE))
        context = create_context(display, config, None, (I * 1)(EGL_NONE))
        if not surface or not context or not make_current(display, surface, surface, context):
            raise RuntimeError("failed to create or bind EGL OpenGL context")
        run_gl(gl)
    finally:
        if context or surface:
            make_current(display, None, None, None)
        if context:
            bind(egl, "eglDestroyContext", U, P, P)(display, context)
        if surface:
            bind(egl, "eglDestroySurface", U, P, P)(display, surface)
        terminate(display)


def run_osmesa():
    # The library exports both OSMesa context functions and the OpenGL entry points.
    gl = C.CDLL(ctypes.util.find_library("OSMesa") or "libOSMesa.so.8")
    I, U, P = C.c_int, C.c_uint, C.c_void_p
    create = bind(gl, "OSMesaCreateContextExt", P, U, I, I, I, P)
    make_current = bind(gl, "OSMesaMakeCurrent", I, P, P, U, I, I)
    destroy = bind(gl, "OSMesaDestroyContext", None, P)
    context = create(0x1908, 16, 0, 0, None)  # RGBA, depth bits
    if not context:
        raise RuntimeError("could not create an OSMesa context")
    pixels = (C.c_ubyte * (W * H * 4))()
    try:
        if not make_current(context, pixels, 0x1401, W, H):
            raise RuntimeError("could not bind the OSMesa context")
        run_gl(gl)
    finally:
        destroy(context)


def run_gl(gl):
    I, U, P, F = C.c_int, C.c_uint, C.c_void_p, C.c_float
    get_string = bind(gl, "glGetString", C.c_char_p, U)
    strings = {}
    for name, key in (("vendor", 0x1F00), ("renderer", 0x1F01),
                      ("GL", 0x1F02), ("GLSL", 0x8B8C)):
        raw = get_string(key)
        strings[name] = raw.decode("ascii", "replace") if raw else ""
        print(f"{name}: {strings[name]}", flush=True)
    if "llvmpipe" not in strings["renderer"].lower():
        raise RuntimeError("renderer is not llvmpipe; refusing to report a software test")
    if not version_at_least(strings["GL"], 2, 1) or not version_at_least(strings["GLSL"], 1, 20):
        raise RuntimeError("OpenGL 2.1 and GLSL 1.20 are required")
    if not re.match(r"^2\.1(?:\D|$)", strings["GL"]) or not re.match(r"^1\.20(?:\D|$)", strings["GLSL"]):
        raise RuntimeError("set MESA_GL_VERSION_OVERRIDE=2.1 and MESA_GLSL_VERSION_OVERRIDE=120")
    get_integer = bind(gl, "glGetIntegerv", None, U, C.POINTER(I))
    for name, key, minimum in (("max_texture_size", 0x0D33, 4),
                               ("max_vertex_attribs", 0x8869, 4)):
        value = I()
        get_integer(key, C.byref(value))
        print(f"{name}: {value.value}", flush=True)
        if value.value < minimum:
            raise RuntimeError(f"{name} is below the prototype's requirement {minimum}")

    create_shader = bind(gl, "glCreateShader", U, U)
    shader_source = bind(gl, "glShaderSource", None, U, I, C.POINTER(C.c_char_p), C.POINTER(I))
    compile_shader = bind(gl, "glCompileShader", None, U)
    get_shader = bind(gl, "glGetShaderiv", None, U, U, C.POINTER(I))
    shader_log = bind(gl, "glGetShaderInfoLog", None, U, I, C.POINTER(I), P)
    delete_shader = bind(gl, "glDeleteShader", None, U)
    create_program = bind(gl, "glCreateProgram", U)
    attach = bind(gl, "glAttachShader", None, U, U)
    link = bind(gl, "glLinkProgram", None, U)
    get_program = bind(gl, "glGetProgramiv", None, U, U, C.POINTER(I))
    program_log = bind(gl, "glGetProgramInfoLog", None, U, I, C.POINTER(I), P)
    delete_program = bind(gl, "glDeleteProgram", None, U)
    shader_dir = Path(__file__).resolve().parents[1] / "crates/horizon-legacy-gl/src"
    shaders, program = [], 0
    buffers, textures = (U * 1)(), (U * 1)()
    try:
        for kind, source in (
            (0x8B31, (shader_dir / "terrain.vert").read_bytes()),
            (0x8B30, (shader_dir / "terrain.frag").read_bytes()),
        ):
            shader = create_shader(kind)
            if not shader:
                raise RuntimeError("glCreateShader failed")
            shaders.append(shader)
            text = C.c_char_p(source)
            shader_source(shader, 1, C.byref(text), None)
            compile_shader(shader)
            ok = I()
            get_shader(shader, 0x8B81, C.byref(ok))
            if not ok.value:
                log = C.create_string_buffer(4096)
                shader_log(shader, len(log), None, log)
                raise RuntimeError("shader compilation failed: " + log.value.decode(errors="replace"))
        program = create_program()
        if not program:
            raise RuntimeError("glCreateProgram failed")
        for shader in shaders:
            attach(program, shader)
        attrib_location = bind(gl, "glBindAttribLocation", None, U, U, C.c_char_p)
        for index, name in enumerate((b"position", b"sprite_uv", b"atlas_rect", b"vertex_color")):
            attrib_location(program, index, name)
        link(program)
        ok = I()
        get_program(program, 0x8B82, C.byref(ok))
        if not ok.value:
            log = C.create_string_buffer(4096)
            program_log(program, len(log), None, log)
            raise RuntimeError("program link failed: " + log.value.decode(errors="replace"))
        bind(gl, "glUseProgram", None, U)(program)
        get_uniform = bind(gl, "glGetUniformLocation", I, U, C.c_char_p)
        location = get_uniform(program, b"atlas")
        if location < 0:
            raise RuntimeError("texture uniform missing")
        bind(gl, "glUniform1i", None, I, I)(location, 0)
        matrix = get_uniform(program, b"view_projection")
        if matrix < 0:
            raise RuntimeError("view_projection uniform missing")
        identity = (F * 16)(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)
        bind(gl, "glUniformMatrix4fv", None, I, I, C.c_ubyte, C.POINTER(F))(matrix, 1, 0, identity)
        bind(gl, "glUniform1f", None, I, F)(get_uniform(program, b"alpha_cutoff"), 0)
        # UV 1.5 wraps to 0.5; the sprite is a 1x1 rectangle inside a 4x4 atlas.
        bind(gl, "glVertexAttrib2f", None, U, F, F)(1, 1.5, 1.5)
        attrib4 = bind(gl, "glVertexAttrib4f", None, U, F, F, F, F)
        attrib4(2, 0.25, 0.25, 0.25, 0.25)
        attrib4(3, 1, 1, 1, 1)

        bind(gl, "glGenBuffers", None, I, C.POINTER(U))(1, buffers)
        bind(gl, "glBindBuffer", None, U, U)(0x8892, buffers[0])
        vertices = (F * 6)(-0.8, -0.8, 0.8, -0.8, 0.0, 0.8)
        bind(gl, "glBufferData", None, U, C.c_ssize_t, P, U)(0x8892, C.sizeof(vertices), vertices, 0x88E4)
        bind(gl, "glEnableVertexAttribArray", None, U)(0)
        bind(gl, "glVertexAttribPointer", None, U, I, U, C.c_ubyte, I, P)(0, 2, 0x1406, 0, 0, None)
        bind(gl, "glGenTextures", None, I, C.POINTER(U))(1, textures)
        bind(gl, "glBindTexture", None, U, U)(0x0DE1, textures[0])
        parameter = bind(gl, "glTexParameteri", None, U, U, I)
        parameter(0x0DE1, 0x2801, 0x2600)  # minification: nearest, no mipmaps
        parameter(0x0DE1, 0x2800, 0x2600)
        atlas_pixels = [0, 255, 0, 255] * 16
        atlas_pixels[(1 * 4 + 1) * 4:(1 * 4 + 1) * 4 + 4] = [200, 40, 20, 128]
        texels = (C.c_ubyte * len(atlas_pixels))(*atlas_pixels)
        bind(gl, "glTexImage2D", None, U, I, I, I, I, I, U, U, P)(0x0DE1, 0, 0x1908, 4, 4, 0, 0x1908, 0x1401, texels)
        bind(gl, "glViewport", None, I, I, I, I)(0, 0, W, H)
        bind(gl, "glClearColor", None, F, F, F, F)(0, 0, 0, 1)
        bind(gl, "glClear", None, U)(0x4000 | 0x0100)
        enable = bind(gl, "glEnable", None, U)
        enable(0x0BE2)  # blend
        enable(0x0B71)  # depth test
        bind(gl, "glBlendFunc", None, U, U)(0x0302, 0x0303)
        bind(gl, "glDrawArrays", None, U, I, I)(0x0004, 0, 3)
        read = bind(gl, "glReadPixels", None, I, I, I, I, U, U, P)
        center, corner = (C.c_ubyte * 4)(), (C.c_ubyte * 4)()
        read(W // 2, H // 2, 1, 1, 0x1908, 0x1401, center)
        read(0, 0, 1, 1, 0x1908, 0x1401, corner)
        error = bind(gl, "glGetError", U)()
        print(f"center={list(center)} corner={list(corner)} gl_error=0x{error:04x}")
        if error or not check_pixels(center, corner):
            raise RuntimeError("rendered pixels or GL state do not match the expected result")

        # Draw a green triangle farther away. A working depth test keeps the
        # previously rendered red pixel, rather than blending green over it.
        green = (C.c_ubyte * 4)(0, 255, 0, 255)
        bind(gl, "glTexSubImage2D", None, U, I, I, I, I, I, U, U, P)(0x0DE1, 0, 1, 1, 1, 1, 0x1908, 0x1401, green)
        bind(gl, "glDepthRange", None, C.c_double, C.c_double)(0.75, 1.0)
        bind(gl, "glDrawArrays", None, U, I, I)(0x0004, 0, 3)
        read(W // 2, H // 2, 1, 1, 0x1908, 0x1401, center)
        if not check_pixels(center, corner):
            raise RuntimeError("depth test did not preserve the nearer triangle")

        # Restore the partially transparent texel and check shader cutout.
        red = (C.c_ubyte * 4)(200, 40, 20, 128)
        bind(gl, "glTexSubImage2D", None, U, I, I, I, I, I, U, U, P)(0x0DE1, 0, 1, 1, 1, 1, 0x1908, 0x1401, red)
        bind(gl, "glClear", None, U)(0x4000 | 0x0100)
        bind(gl, "glUniform1f", None, I, F)(get_uniform(program, b"alpha_cutoff"), 0.75)
        bind(gl, "glDrawArrays", None, U, I, I)(0x0004, 0, 3)
        read(W // 2, H // 2, 1, 1, 0x1908, 0x1401, center)
        error = bind(gl, "glGetError", U)()
        if error or any(v > 3 for v in center[:3]):
            raise RuntimeError(f"cutout failed: center={list(center)} GL error=0x{error:04x}")
        print("PASS: legacy terrain shaders, atlas UV wrap, blending, depth and cutout on llvmpipe")
    finally:
        if buffers[0]:
            bind(gl, "glDeleteBuffers", None, I, C.POINTER(U))(1, buffers)
        if textures[0]:
            bind(gl, "glDeleteTextures", None, I, C.POINTER(U))(1, textures)
        if program:
            delete_program(program)
        for shader in shaders:
            delete_shader(shader)


if __name__ == "__main__":
    try:
        main()
    except (OSError, AttributeError, RuntimeError) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        sys.exit(1)
