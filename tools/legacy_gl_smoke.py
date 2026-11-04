#!/usr/bin/env python3
"""Headless desktop OpenGL 2 terrain-shader smoke test on Linux/Mesa.

Requires libEGL, Mesa's desktop GL driver and PyOpenGL. It renders to an EGL
pbuffer, not an FBO, so framebuffer extensions are not a prerequisite. This is
an offline development test, not a Windows executable or an X1650 test.

    LIBGL_ALWAYS_SOFTWARE=1 MESA_GL_VERSION_OVERRIDE=2.0 \
      MESA_GLSL_VERSION_OVERRIDE=110 python3 tools/legacy_gl_smoke.py
"""

import ctypes
import ctypes.util
import os
from pathlib import Path
import sys

os.environ.setdefault("EGL_PLATFORM", "surfaceless")
os.environ.setdefault("PYOPENGL_PLATFORM", "egl")

ROOT = Path(__file__).resolve().parents[1]
SHADER_DIR = ROOT / "crates/horizon-client-render/src"


def create_context():
    name = ctypes.util.find_library("EGL")
    if not name:
        raise RuntimeError("libEGL was not found; install Mesa EGL and its desktop GL driver")
    egl = ctypes.CDLL(name)
    i, p = ctypes.c_int, ctypes.c_void_p

    def bind(name, result, *args):
        func = getattr(egl, name)
        func.restype, func.argtypes = result, list(args)
        return func

    get_display = bind("eglGetDisplay", p, p)
    initialize = bind("eglInitialize", i, p, ctypes.POINTER(i), ctypes.POINTER(i))
    bind_api = bind("eglBindAPI", i, i)
    choose = bind("eglChooseConfig", i, p, ctypes.POINTER(i), ctypes.POINTER(p), i, ctypes.POINTER(i))
    surface_fn = bind("eglCreatePbufferSurface", p, p, p, ctypes.POINTER(i))
    context_fn = bind("eglCreateContext", p, p, p, p, ctypes.POINTER(i))
    make_current = bind("eglMakeCurrent", i, p, p, p, p)
    destroy_context = bind("eglDestroyContext", i, p, p)
    destroy_surface = bind("eglDestroySurface", i, p, p)
    terminate = bind("eglTerminate", i, p)
    error = bind("eglGetError", i)

    def check(ok, operation):
        if not ok:
            raise RuntimeError(f"{operation} failed (EGL error 0x{error():04x})")
        return ok

    display = check(get_display(None), "eglGetDisplay")
    surface = context = None
    try:
        major, minor = i(), i()
        check(initialize(display, ctypes.byref(major), ctypes.byref(minor)), "eglInitialize")
        check(bind_api(0x30A2), "eglBindAPI(EGL_OPENGL_API)")
        # Pbuffer, OpenGL, RGBA8, 16-bit depth.
        attrs = (i * 17)(0x3033, 1, 0x3040, 8, 0x3024, 8, 0x3023, 8,
                         0x3022, 8, 0x3021, 8, 0x3025, 16, 0x3038, 0x3038, 0x3038)
        config, count = p(), i()
        check(choose(display, attrs, ctypes.byref(config), 1, ctypes.byref(count)), "eglChooseConfig")
        check(count.value > 0, "no suitable EGL config")
        surface_attrs = (i * 5)(0x3057, 8, 0x3056, 8, 0x3038)
        surface = check(surface_fn(display, config, surface_attrs), "eglCreatePbufferSurface")
        context_attrs = (i * 3)(0x3098, 2, 0x3038)  # EGL_CONTEXT_CLIENT_VERSION = 2
        context = check(context_fn(display, config, None, context_attrs), "eglCreateContext")
        check(make_current(display, surface, surface, context), "eglMakeCurrent")
    except BaseException:
        if context:
            destroy_context(display, context)
        if surface:
            destroy_surface(display, surface)
        terminate(display)
        raise

    def close():
        make_current(display, None, None, None)
        destroy_context(display, context)
        destroy_surface(display, surface)
        terminate(display)

    return close


def run():
    close = create_context()
    try:
        from OpenGL import GL
        from OpenGL.GL import shaders

        for label, token in [("vendor", GL.GL_VENDOR), ("renderer", GL.GL_RENDERER),
                             ("version", GL.GL_VERSION), ("GLSL", GL.GL_SHADING_LANGUAGE_VERSION)]:
            value = GL.glGetString(token)
            if not value:
                raise RuntimeError(f"GL {label} not available")
            print(f"{label}: {value.decode('ascii', errors='replace')}")

        vs = (SHADER_DIR / "legacy_gl_terrain.vert").read_text(encoding="utf-8")
        fs = (SHADER_DIR / "legacy_gl_terrain.frag").read_text(encoding="utf-8")
        program = shaders.compileProgram(shaders.compileShader(vs, GL.GL_VERTEX_SHADER),
                                         shaders.compileShader(fs, GL.GL_FRAGMENT_SHADER))
        GL.glUseProgram(program)
        # Two triangles: position xyz, sprite uv, lightmap uv, color rgba.
        def vertex(x, y, u, v):
            return [x, y, 0.0, u, v, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0]

        values = (vertex(-1, -1, 0, 0) + vertex(1, -1, 1, 0) + vertex(1, 1, 1, 1)
                  + vertex(-1, -1, 0, 0) + vertex(1, 1, 1, 1) + vertex(-1, 1, 0, 1))
        data = (ctypes.c_float * len(values))(*values)
        vbo = GL.glGenBuffers(1)
        GL.glBindBuffer(GL.GL_ARRAY_BUFFER, vbo)
        GL.glBufferData(GL.GL_ARRAY_BUFFER, ctypes.sizeof(data), data, GL.GL_STATIC_DRAW)
        for name, width, offset in [("a_position", 3, 0), ("a_uv", 2, 3),
                                    ("a_light_uv", 2, 5), ("a_color", 4, 7)]:
            location = GL.glGetAttribLocation(program, name)
            if location < 0:
                raise RuntimeError(f"missing shader attribute {name}")
            GL.glEnableVertexAttribArray(location)
            GL.glVertexAttribPointer(location, width, GL.GL_FLOAT, False, 44, ctypes.c_void_p(offset * 4))
        identity = (ctypes.c_float * 16)(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)
        GL.glUniformMatrix4fv(GL.glGetUniformLocation(program, "u_view_projection"), 1, False, identity)
        GL.glUniform3f(GL.glGetUniformLocation(program, "u_section_origin"), 0, 0, 0)
        GL.glUniform1f(GL.glGetUniformLocation(program, "u_alpha_cutoff"), 0.5)

        textures = GL.glGenTextures(2)
        for unit, texture, uniform, width, pixels in [
            (0, textures[0], "u_atlas", 2, bytes([255, 0, 0, 255, 255, 0, 0, 0])),
            (1, textures[1], "u_lightmap", 1, bytes([255, 255, 255, 255])),
        ]:
            GL.glActiveTexture(GL.GL_TEXTURE0 + unit)
            GL.glBindTexture(GL.GL_TEXTURE_2D, texture)
            GL.glTexParameteri(GL.GL_TEXTURE_2D, GL.GL_TEXTURE_MIN_FILTER, GL.GL_NEAREST)
            GL.glTexParameteri(GL.GL_TEXTURE_2D, GL.GL_TEXTURE_MAG_FILTER, GL.GL_NEAREST)
            GL.glTexImage2D(GL.GL_TEXTURE_2D, 0, GL.GL_RGBA, width, 1, 0, GL.GL_RGBA, GL.GL_UNSIGNED_BYTE, pixels)
            GL.glUniform1i(GL.glGetUniformLocation(program, uniform), unit)
        GL.glViewport(0, 0, 8, 8)
        GL.glClearColor(0, 0, 1, 1)
        GL.glClear(GL.GL_COLOR_BUFFER_BIT | GL.GL_DEPTH_BUFFER_BIT)
        GL.glDrawArrays(GL.GL_TRIANGLES, 0, 6)
        GL.glFinish()
        left = bytes(GL.glReadPixels(1, 4, 1, 1, GL.GL_RGBA, GL.GL_UNSIGNED_BYTE))
        right = bytes(GL.glReadPixels(6, 4, 1, 1, GL.GL_RGBA, GL.GL_UNSIGNED_BYTE))
        if left != bytes([255, 0, 0, 255]) or right != bytes([0, 0, 255, 255]):
            raise RuntimeError(f"unexpected pixels: opaque={list(left)}, cutout={list(right)}")
        print("PASS: GLSL 1.10, VBO, two textures and alpha discard")
    finally:
        close()


if __name__ == "__main__":
    try:
        run()
    except Exception as exc:
        print(f"legacy GL smoke test failed: {exc}", file=sys.stderr)
        sys.exit(1)
