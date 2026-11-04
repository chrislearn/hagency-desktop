"""Report Direct3D device creation on the disposable Windows installer runner."""
import ctypes as c
from ctypes import wintypes as w
import json
import uuid


def device_probe():
    class Guid(c.Structure):
        _fields_ = [('data', c.c_ubyte * 16)]

    def method(pointer, index, result, *arguments):
        table = c.cast(pointer, c.POINTER(c.POINTER(c.c_void_p))).contents
        return c.WINFUNCTYPE(result, c.c_void_p, *arguments)(table[index])

    def release(pointer):
        if pointer:
            method(pointer, 2, w.ULONG)(pointer)

    create = c.WinDLL('d3d11').D3D11CreateDevice
    create.restype = c.c_long
    create.argtypes = [c.c_void_p, c.c_uint, c.c_void_p, c.c_uint,
                        c.POINTER(c.c_uint), c.c_uint, c.c_uint,
                        c.POINTER(c.c_void_p), c.POINTER(c.c_uint),
                        c.POINTER(c.c_void_p)]
    factory, adapter = c.c_void_p(), c.c_void_p()
    iid = Guid((c.c_ubyte * 16).from_buffer_copy(
        uuid.UUID('50c83a1c-e072-4c48-87b0-3630fa36a6d0').bytes_le))
    factory_create = c.WinDLL('dxgi').CreateDXGIFactory2
    factory_create.restype = c.c_long
    factory_create.argtypes = [c.c_uint, c.POINTER(Guid), c.POINTER(c.c_void_p)]
    results = []
    try:
        status = factory_create(0, c.byref(iid), c.byref(factory))
        if status == 0:
            status = method(factory, 7, c.c_long, c.c_uint,
                            c.POINTER(c.c_void_p))(factory, 0, c.byref(adapter))
        results.append(dict(factoryAdapterStatus=f'{status & 0xffffffff:08x}'))
        for name, pointer, driver in [('hardware', None, 1), ('warp', None, 5),
                                      ('adapter0', adapter, 0)]:
            if name == 'adapter0' and not adapter:
                continue
            for flags in (0x20, 0x820):
                device, context = c.c_void_p(), c.c_void_p()
                level = c.c_uint(0xb000)
                actual = c.c_uint()
                try:
                    status = create(pointer, driver, None, flags, c.byref(level),
                                    1, 7, c.byref(device), c.byref(actual), c.byref(context))
                    results.append(dict(driver=name, flags=f'{flags:x}',
                                        status=f'{status & 0xffffffff:08x}',
                                        featureLevel=f'{actual.value:x}'))
                finally:
                    release(context)
                    release(device)
    finally:
        release(adapter)
        release(factory)
    return json.dumps(results, indent=2)
