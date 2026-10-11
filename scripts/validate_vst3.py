#!/usr/bin/env python3
"""Headless Windows x64 VST3 factory and actual stereo processing checks.

The COM vtables and C layouts follow pinned vst3-sys b3ff4d7. This exercises
stopped-transport previews and playback alignment without opening an editor;
it does not substitute for FL Studio routing or GUI integration tests.
"""
import argparse
import ctypes as C
from concurrent.futures import ThreadPoolExecutor
import math
import sys
from pathlib import Path
from uuid import UUID


VOID, I32, U32, U64, I64, F64 = C.c_void_p, C.c_int32, C.c_uint32, C.c_uint64, C.c_int64, C.c_double
IID = C.c_ubyte * 16
U16String = C.c_uint16 * 128


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def guid(value):
    # On Windows, the first GUID fields use COM little-endian byte order.
    return IID.from_buffer_copy(UUID(value).bytes_le)


def method(pointer, index, result, *args):
    table = C.cast(pointer, C.POINTER(C.POINTER(VOID))).contents
    return C.WINFUNCTYPE(result, VOID, *args)(table[index])


def query(pointer, iid):
    output = VOID()
    require(method(pointer, 0, I32, C.POINTER(IID), C.POINTER(VOID))(
        pointer, C.byref(iid), C.byref(output)) == 0 and output.value,
        "Required VST3 COM interface is unavailable")
    return output.value


def release(pointer):
    if pointer:
        method(pointer, 2, U32)(pointer)


class FactoryInfo(C.Structure):
    _fields_ = [("vendor", C.c_char * 64), ("url", C.c_char * 256),
                ("email", C.c_char * 128), ("flags", C.c_int32)]


class ClassInfo(C.Structure):
    _fields_ = [("cid", C.c_ubyte * 16), ("cardinality", C.c_int32),
                ("category", C.c_char * 32), ("name", C.c_char * 64)]


class BusInfo(C.Structure):
    _fields_ = [("media_type", I32), ("direction", I32), ("channel_count", I32),
                ("name", U16String), ("bus_type", I32), ("flags", U32)]


class ParameterInfo(C.Structure):
    _fields_ = [("id", U32), ("title", U16String), ("short_title", U16String),
                ("units", U16String), ("step_count", I32),
                ("default_normalized_value", F64), ("unit_id", I32), ("flags", I32)]


class ProcessSetup(C.Structure):
    _fields_ = [("process_mode", I32), ("symbolic_sample_size", I32),
                ("max_samples_per_block", I32), ("sample_rate", F64)]


class AudioBusBuffers(C.Structure):
    _fields_ = [("num_channels", I32), ("silence_flags", U64),
                ("buffers", C.POINTER(C.POINTER(C.c_float)))]


class Chord(C.Structure):
    _fields_ = [("key_note", C.c_uint8), ("root_note", C.c_uint8),
                ("chord_mask", C.c_int16)]


class FrameRate(C.Structure):
    _fields_ = [("frames_per_second", U32), ("flags", U32)]


class ProcessContext(C.Structure):
    _fields_ = [("state", U32), ("sample_rate", F64), ("project_time_samples", I64),
                ("system_time", I64), ("continuous_time_samples", I64),
                ("project_time_music", F64), ("bar_position_music", F64),
                ("cycle_start_music", F64), ("cycle_end_music", F64), ("tempo", F64),
                ("time_sig_num", I32), ("time_sig_den", I32), ("chord", Chord),
                ("smpte_offset_subframes", I32), ("frame_rate", FrameRate),
                ("samples_to_next_clock", I64)]


class ProcessData(C.Structure):
    _fields_ = [("process_mode", I32), ("symbolic_sample_size", I32),
                ("num_samples", I32), ("num_inputs", I32), ("num_outputs", I32),
                ("inputs", C.POINTER(AudioBusBuffers)), ("outputs", C.POINTER(AudioBusBuffers)),
                ("input_param_changes", VOID), ("output_param_changes", VOID),
                ("input_events", VOID), ("output_events", VOID),
                ("context", C.POINTER(ProcessContext))]


def check_abi():
    expected = {FactoryInfo: 452, ClassInfo: 116, BusInfo: 276, ParameterInfo: 792,
                ProcessSetup: 24, AudioBusBuffers: 24, Chord: 4, FrameRate: 8,
                ProcessContext: 112, ProcessData: 80}
    for structure, size in expected.items():
        require(C.sizeof(structure) == size, f"ABI size mismatch: {structure.__name__}")
    require(ProcessContext.project_time_music.offset == 40 and ProcessData.inputs.offset == 24,
            "Unexpected VST3 C ABI field alignment")


class ProcessingHost:
    def __init__(self, factory, class_id):
        self.component = self.processor = self.controller = None
        self.initialized = self.active = self.processing = False
        self.executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="vst3-audio")
        self.position = 0
        component_iid = guid("E831FF31-F2D5-4301-928E-BBEE25697802")
        component = VOID()
        require(method(factory, 6, I32, C.POINTER(IID), C.POINTER(IID), C.POINTER(VOID))(
            factory, C.byref(class_id), C.byref(component_iid), C.byref(component)) == 0
            and component.value, "VST3 component creation failed")
        self.component = component.value
        try:
            self.processor = query(self.component, guid("42043F99-B7DA-453C-A569-E79D9AAEC33D"))
            self.controller = query(self.component, guid("DCD7BBE3-7742-448D-A874-AACC979C759E"))
            # The pinned NIH wrapper does not require an IHostApplication.
            require(method(self.component, 3, I32, VOID)(self.component, None) == 0,
                    "VST3 initialize failed")
            self.initialized = True
            for direction in (0, 1):
                require(method(self.component, 7, I32, I32, I32)(
                    self.component, 0, direction) == 1, "Expected one main audio bus")
                info = BusInfo()
                require(method(self.component, 8, I32, I32, I32, I32, C.POINTER(BusInfo))(
                    self.component, 0, direction, 0, C.byref(info)) == 0
                    and info.channel_count == 2 and info.bus_type == 0, "Expected a stereo main bus")
            arrangements = (U64 * 1)(3)  # kStereo = left | right
            require(method(self.processor, 3, I32, C.POINTER(U64), I32, C.POINTER(U64), I32)(
                self.processor, arrangements, 1, arrangements, 1) == 0, "Stereo layout rejected")
            require(method(self.processor, 5, I32, I32)(self.processor, 0) == 0,
                    "32-bit processing is unsupported")
            self.infos = {}
            count = method(self.controller, 8, I32)(self.controller)
            for index in range(count):
                info = ParameterInfo()
                require(method(self.controller, 9, I32, I32, C.POINTER(ParameterInfo))(
                    self.controller, index, C.byref(info)) == 0, "Parameter enumeration failed")
                name = bytes(info.title).decode("utf-16-le").split("\0", 1)[0]
                require(name not in self.infos, f"Duplicate VST3 parameter {name}")
                self.infos[name] = info
            require({"Tempo sync", "Volume point 00", "Volume interpolation", "De-click"}
                    .issubset(self.infos), "Required VST3 shaping parameters missing")
            print(f"PASS VST3 component/controller/audio interfaces; stereo layout; {count} parameters")
        except Exception:
            self.close()
            raise

    def set(self, updates):
        require(not self.active, "Set test parameters before activation")
        for name, value in updates.items():
            parameter = self.infos[name]
            require(0.0 <= value <= 1.0, f"Invalid normalized value for {name}")
            require(method(self.controller, 15, I32, U32, F64)(
                self.controller, parameter.id, value) == 0, f"VST3 parameter edit failed: {name}")
            actual = method(self.controller, 14, F64, U32)(self.controller, parameter.id)
            require(abs(actual - value) < 1e-6, f"VST3 parameter value did not change: {name}")

    def activate(self):
        self.position = 0
        setup = ProcessSetup(0, 0, 512, 48000.0)
        require(method(self.processor, 7, I32, C.POINTER(ProcessSetup))(
            self.processor, C.byref(setup)) == 0, "VST3 setupProcessing failed")
        for direction in (0, 1):
            require(method(self.component, 10, I32, I32, I32, I32, C.c_uint8)(
                self.component, 0, direction, 0, 1) == 0, "VST3 bus activation failed")
        require(method(self.component, 11, I32, C.c_uint8)(self.component, 1) == 0,
                "VST3 setActive failed")
        self.active = True
        require(self.executor.submit(method(self.processor, 8, I32, C.c_uint8),
                                     self.processor, 1).result() == 0, "VST3 setProcessing failed")
        self.processing = True

    def block(self, count=512, tempo=120.0, beat=0.0, playing=False, input_values=(0.25, 0.15)):
        require(self.processing and 1 <= count <= 512, "Invalid VST3 process call")
        inputs = [(C.c_float * count)(*([value] * count)) for value in input_values]
        outputs = [(C.c_float * count)() for _ in range(2)]
        input_ptrs = (C.POINTER(C.c_float) * 2)(*inputs)
        output_ptrs = (C.POINTER(C.c_float) * 2)(*outputs)
        in_buffer = AudioBusBuffers(2, 3 if input_values == (0.0, 0.0) else 0, input_ptrs)
        out_buffer = AudioBusBuffers(2, 0, output_ptrs)
        context = ProcessContext()
        context.state = (1 << 9) | (1 << 10) | (1 << 13) | (1 << 17)
        if playing:
            context.state |= 1 << 1
        context.sample_rate = 48000.0
        context.project_time_samples = round(beat * 60.0 / tempo * 48000.0)
        context.continuous_time_samples = self.position
        context.project_time_music = beat
        context.tempo = tempo
        context.time_sig_num, context.time_sig_den = 4, 4
        data = ProcessData(0, 0, count, 1, 1, C.pointer(in_buffer), C.pointer(out_buffer),
                           None, None, None, None, C.pointer(context))
        require(self.executor.submit(method(self.processor, 9, I32, C.POINTER(ProcessData)),
                                     self.processor, C.byref(data)).result() == 0,
                "VST3 process failed")
        rendered = [list(channel) for channel in outputs]
        require(all(math.isfinite(value) for channel in rendered for value in channel),
                "Non-finite VST3 output")
        self.position += count
        return rendered

    def deactivate(self):
        if self.processing:
            require(self.executor.submit(method(self.processor, 8, I32, C.c_uint8),
                                         self.processor, 0).result() == 0,
                    "VST3 stopProcessing failed")
            self.processing = False
        if self.active:
            require(method(self.component, 11, I32, C.c_uint8)(self.component, 0) == 0,
                    "VST3 deactivation failed")
            self.active = False

    def close(self):
        self.deactivate()
        if self.initialized:
            method(self.component, 4, I32)(self.component)
            self.initialized = False
        for pointer in (self.controller, self.processor, self.component):
            release(pointer)
        self.component = self.processor = self.controller = None
        self.executor.shutdown(wait=True)


def render_preview(host, frames, sizes, input_values=(0.25, 0.15), move_playhead=False):
    output = [[], []]
    host.activate()
    try:
        processed = index = 0
        while processed < frames:
            count = min(sizes[index % len(sizes)], frames - processed)
            beat = 88.375 if move_playhead and index % 2 else 0.0
            block = host.block(count, tempo=127.0, beat=beat, input_values=input_values)
            for target, channel in zip(output, block):
                target.extend(channel)
            processed += count
            index += 1
    finally:
        host.deactivate()
    return output


def same_audio(actual, expected, message):
    require(all(len(a) == len(b) for a, b in zip(actual, expected)), f"{message}: length mismatch")
    error = max(abs(a - b) for channel_a, channel_b in zip(actual, expected)
                for a, b in zip(channel_a, channel_b))
    require(error < 2e-6, f"{message}: maximum sample error {error:.9f}")


def validate_processing(host):
    host.activate()
    for _ in range(32):
        host.block(input_values=(0.0, 0.0))
    note = host.block(256)
    require(max(note[0]) > 0.20 and max(note[1]) > 0.12,
            "VST3 Sync Pump muted a stopped note after silent preroll")
    host.deactivate()
    fixed = render_preview(host, 48001, (512,))
    varied = render_preview(host, 48001, (1, 17, 64, 127, 512))
    moved = render_preview(host, 48001, (1, 17, 64, 127, 512), move_playhead=True)
    require(min(fixed[0]) < 0.05 and max(fixed[0]) > 0.20, "VST3 stopped preview does not pump")
    same_audio(varied, fixed, "VST3 stopped preview depends on buffer sizes")
    same_audio(moved, fixed, "VST3 stopped playhead resets preview phase")
    print("PASS VST3 stopped Sync Pump: note after silent preroll; varied buffers; fixed/moving playhead")

    sizes = (17, 512, 64, 1, 127)
    first = render_preview(host, 24001, sizes, (0.25, 0.15))
    second = render_preview(host, 24001, sizes, (0.175, 0.225))
    combined = render_preview(host, 24001, sizes, (0.425, 0.375))
    expected = [[a + b for a, b in zip(left, right)] for left, right in zip(first, second)]
    same_audio(combined, expected, "VST3 summed stereo main-bus sources differ")
    print("PASS VST3 two source signals summed into the stereo main bus")

    ramp = {f"Volume point {index:02}": index / 16 for index in range(17)}
    ramp.update({"Volume interpolation": 0.0, "De-click": 0.0})
    host.set(ramp)
    host.activate()
    for _ in range(8):
        host.block(127)
    for beat, gain in ((88.375, 0.375), (-0.25, 0.75), (0.125, 0.125)):
        block = host.block(17, beat=beat, playing=True)
        require(all(abs(channel[0] - level * gain) < 1e-6
                    for channel, level in zip(block, (0.25, 0.15))),
                f"VST3 resumed playback/seek did not align first sample at beat {beat}")
    host.deactivate()
    print("PASS VST3 preview-to-playback exact phase alignment and positive/negative seeks")


def validate(path):
    if sys.platform != "win32" or C.sizeof(C.c_void_p) != 8:
        raise RuntimeError("This validation requires Windows x64 Python.")
    check_abi()
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
    require(library.InitDll(), "Module initialization failed")
    factory = library.GetPluginFactory()
    require(factory, "No plugin factory")
    try:
        info = FactoryInfo()
        require(method(factory, 3, I32, C.POINTER(FactoryInfo))(factory, C.byref(info)) == 0,
                "VST3 factory information unavailable")
        require(info.vendor == b"PRISMM", "Unexpected VST3 vendor")
        count = method(factory, 4, I32)(factory)
        require(count == 1, f"Expected one ShapeRack class, found {count}")
        plugin = ClassInfo()
        require(method(factory, 5, I32, I32, C.POINTER(ClassInfo))(
            factory, 0, C.byref(plugin)) == 0, "VST3 class information unavailable")
        require(plugin.name == b"ShapeRack" and plugin.category == b"Audio Module Class",
                "Unexpected VST3 plugin descriptor")
        require(any(plugin.cid), "Class ID must be nonzero")
        print(f"PASS: Windows VST3 module loaded; factory exposes {plugin.name.decode()} by {info.vendor.decode()}.")
        host = ProcessingHost(factory, plugin.cid)
        try:
            validate_processing(host)
        finally:
            host.close()
    finally:
        release(factory)
        library.ExitDll()
    print("ShapeRack VST3 integration validation passed.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", help="ShapeRack.vst3 bundle or inner Windows module")
    validate(parser.parse_args().path)
