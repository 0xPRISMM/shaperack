#!/usr/bin/env python3
"""Windows x64 VST3 factory/export smoke check, not a full DAW validator."""
import argparse
import ctypes as C
import sys
from pathlib import Path


class FactoryInfo(C.Structure):
    _fields_ = [("vendor", C.c_char * 64), ("url", C.c_char * 256),
                ("email", C.c_char * 128), ("flags", C.c_int32)]


class ClassInfo(C.Structure):
    _fields_ = [("cid", C.c_ubyte * 16), ("cardinality", C.c_int32),
                ("category", C.c_char * 32), ("name", C.c_char * 64)]


def validate(path):
    if sys.platform != "win32" or C.sizeof(C.c_void_p) != 8:
        raise RuntimeError("This factory check requires Windows x64 Python.")
    path = Path(path).resolve(strict=True)
    if path.is_dir():
        path = path / "Contents" / "x86_64-win" / "ShapeRack.vst3"
    library = C.WinDLL(str(path))
    library.InitDll.argtypes = []
    library.InitDll.restype = C.c_bool
    library.ExitDll.argtypes = []
    library.ExitDll.restype = C.c_bool
    library.GetPluginFactory.argtypes = []
    library.GetPluginFactory.restype = C.c_void_p
    assert library.InitDll(), "Module initialization failed"
    factory = library.GetPluginFactory()
    assert factory, "No plugin factory"
    table = C.cast(factory, C.POINTER(C.POINTER(C.c_void_p))).contents
    method = lambda index, result, *args: C.WINFUNCTYPE(result, C.c_void_p, *args)(table[index])
    release = method(2, C.c_uint32)
    try:
        info = FactoryInfo()
        assert method(3, C.c_int32, C.POINTER(FactoryInfo))(factory, C.byref(info)) == 0
        assert info.vendor == b"PRISMM"
        count = method(4, C.c_int32)(factory)
        assert count == 1, f"Expected one ShapeRack class, found {count}"
        plugin = ClassInfo()
        assert method(5, C.c_int32, C.c_int32, C.POINTER(ClassInfo))(factory, 0, C.byref(plugin)) == 0
        assert plugin.name == b"ShapeRack"
        assert plugin.category == b"Audio Module Class"
        assert any(plugin.cid), "Class ID must be nonzero"
        print(f"PASS: Windows VST3 module loaded; factory exposes {plugin.name.decode()} by {info.vendor.decode()}.")
    finally:
        release(factory)
        library.ExitDll()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", help="ShapeRack.vst3 bundle or inner Windows module")
    validate(parser.parse_args().path)
