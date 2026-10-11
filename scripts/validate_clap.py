#!/usr/bin/env python3
"""Exercise the actual ShapeRack CLAP library with a small standard-library host.

Usage: python scripts/validate_clap.py target/bundled/ShapeRack.clap
Run with Python of the same architecture as the plugin (64-bit for release builds).
The C ABI layouts below follow clap-sys 25d7f53 (CLAP 1.2.2), the pinned
NIH-plug dependency, including C calling convention and fixed-point transport.
This is a headless integration test; it does not validate a DAW or the editor.
"""

import argparse
import ctypes as C
from concurrent.futures import ThreadPoolExecutor
import math
from pathlib import Path
import threading


U32, I32, U64, I64 = C.c_uint32, C.c_int32, C.c_uint64, C.c_int64
F64, VOID, CHAR = C.c_double, C.c_void_p, C.c_char_p
FN = C.CFUNCTYPE  # CLAP is extern "C", also on Windows.


class Version(C.Structure):
    _fields_ = [("major", U32), ("minor", U32), ("revision", U32)]


class Descriptor(C.Structure):
    _fields_ = [("version", Version)] + [
        (field, CHAR) for field in (
            "id", "name", "vendor", "url", "manual_url", "support_url",
            "plugin_version", "description",
        )
    ] + [("features", C.POINTER(CHAR))]


class Plugin(C.Structure):
    pass


class Host(C.Structure):
    pass


class EventHeader(C.Structure):
    _fields_ = [("size", U32), ("time", U32), ("space_id", C.c_uint16),
                ("type", C.c_uint16), ("flags", U32)]


class ParamEvent(C.Structure):
    _fields_ = [("header", EventHeader), ("param_id", U32), ("cookie", VOID),
                ("note_id", I32), ("port_index", C.c_int16),
                ("channel", C.c_int16), ("key", C.c_int16), ("value", F64)]


class Transport(C.Structure):
    _fields_ = [("header", EventHeader), ("flags", U32),
                ("song_pos_beats", I64), ("song_pos_seconds", I64),
                ("tempo", F64), ("tempo_inc", F64),
                ("loop_start_beats", I64), ("loop_end_beats", I64),
                ("loop_start_seconds", I64), ("loop_end_seconds", I64),
                ("bar_start", I64), ("bar_number", I32),
                ("tsig_num", C.c_uint16), ("tsig_denom", C.c_uint16)]


class InputEvents(C.Structure):
    pass


class OutputEvents(C.Structure):
    pass


InputEvents._fields_ = [("ctx", VOID),
    ("size", FN(U32, C.POINTER(InputEvents))),
    # Python callbacks cannot return ctypes pointer objects; void* has identical ABI.
    ("get", FN(VOID, C.POINTER(InputEvents), U32))]
OutputEvents._fields_ = [("ctx", VOID),
    ("try_push", FN(C.c_bool, C.POINTER(OutputEvents), C.POINTER(EventHeader)))]


class AudioBuffer(C.Structure):
    _fields_ = [("data32", C.POINTER(C.POINTER(C.c_float))),
                ("data64", C.POINTER(C.POINTER(F64))),
                ("channel_count", U32), ("latency", U32), ("constant_mask", U64)]


class Process(C.Structure):
    _fields_ = [("steady_time", I64), ("frames_count", U32),
                ("transport", C.POINTER(Transport)),
                ("audio_inputs", C.POINTER(AudioBuffer)),
                ("audio_outputs", C.POINTER(AudioBuffer)),
                ("audio_inputs_count", U32), ("audio_outputs_count", U32),
                ("in_events", C.POINTER(InputEvents)),
                ("out_events", C.POINTER(OutputEvents))]


PP, HP = C.POINTER(Plugin), C.POINTER(Host)
Plugin._fields_ = [("desc", C.POINTER(Descriptor)), ("plugin_data", VOID),
    ("init", FN(C.c_bool, PP)), ("destroy", FN(None, PP)),
    ("activate", FN(C.c_bool, PP, F64, U32, U32)),
    ("deactivate", FN(None, PP)), ("start_processing", FN(C.c_bool, PP)),
    ("stop_processing", FN(None, PP)), ("reset", FN(None, PP)),
    ("process", FN(I32, PP, C.POINTER(Process))),
    ("get_extension", FN(VOID, PP, CHAR)), ("on_main_thread", FN(None, PP))]
Host._fields_ = [("version", Version), ("host_data", VOID),
    ("name", CHAR), ("vendor", CHAR), ("url", CHAR), ("host_version", CHAR),
    ("get_extension", FN(VOID, HP, CHAR)), ("request_restart", FN(None, HP)),
    ("request_process", FN(None, HP)), ("request_callback", FN(None, HP))]


class ThreadCheck(C.Structure):
    _fields_ = [("is_main_thread", FN(C.c_bool, HP)),
                ("is_audio_thread", FN(C.c_bool, HP))]


class Entry(C.Structure):
    _fields_ = [("version", Version), ("init", FN(C.c_bool, CHAR)),
                ("deinit", FN(None)), ("get_factory", FN(VOID, CHAR))]


class Factory(C.Structure):
    pass


FP = C.POINTER(Factory)
Factory._fields_ = [("get_plugin_count", FN(U32, FP)),
    ("get_plugin_descriptor", FN(C.POINTER(Descriptor), FP, U32)),
    ("create_plugin", FN(PP, FP, HP, CHAR))]


class ParamInfo(C.Structure):
    _fields_ = [("id", U32), ("flags", U32), ("cookie", VOID),
                ("name", C.c_char * 256), ("module", C.c_char * 1024),
                ("min_value", F64), ("max_value", F64), ("default_value", F64)]


class Params(C.Structure):
    _fields_ = [("count", FN(U32, PP)),
        ("get_info", FN(C.c_bool, PP, U32, C.POINTER(ParamInfo))),
        ("get_value", FN(C.c_bool, PP, U32, C.POINTER(F64))),
        ("value_to_text", FN(C.c_bool, PP, U32, F64, VOID, U32)),
        ("text_to_value", FN(C.c_bool, PP, U32, CHAR, C.POINTER(F64))),
        ("flush", FN(None, PP, C.POINTER(InputEvents), C.POINTER(OutputEvents)))]


class OutputStream(C.Structure):
    pass


class InputStream(C.Structure):
    pass


OutputStream._fields_ = [("ctx", VOID),
    ("write", FN(I64, C.POINTER(OutputStream), VOID, U64))]
InputStream._fields_ = [("ctx", VOID),
    ("read", FN(I64, C.POINTER(InputStream), VOID, U64))]


class State(C.Structure):
    _fields_ = [("save", FN(C.c_bool, PP, C.POINTER(OutputStream))),
                ("load", FN(C.c_bool, PP, C.POINTER(InputStream)))]


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def check_abi():
    require(C.sizeof(VOID) == 8, "Use a 64-bit Python interpreter for the x64 plugin")
    expected = {Version: 12, Descriptor: 88, Plugin: 96, Host: 88, Entry: 40,
                EventHeader: 16, ParamEvent: 56, Transport: 104, AudioBuffer: 32,
                Process: 64, ParamInfo: 1320}
    for structure, size in expected.items():
        require(C.sizeof(structure) == size, f"ABI size mismatch: {structure.__name__}")
    require(ParamEvent.value.offset == 48 and Process.in_events.offset == 48,
            "Unexpected C ABI field alignment")


class EventList:
    """Keep every callback and event alive until the native call finishes."""

    def __init__(self, events=()):
        self.events = sorted(events, key=lambda event: event.header.time)
        self.size = InputEvents._fields_[1][1](lambda _list: len(self.events))
        self.get = InputEvents._fields_[2][1](
            lambda _list, index: C.addressof(self.events[index]) if index < len(self.events) else None)
        self.push = OutputEvents._fields_[1][1](lambda _list, _event: True)
        self.input = InputEvents(None, self.size, self.get)
        self.output = OutputEvents(None, self.push)


class TestHost:
    def __init__(self, path):
        self.main_ident = threading.get_ident()
        self.audio_ident = None
        self.callback_requested = False
        self.executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="clap-audio")
        self.audio_ident = self.executor.submit(threading.get_ident).result()
        self.thread_check = ThreadCheck(
            ThreadCheck._fields_[0][1](lambda _host: threading.get_ident() == self.main_ident),
            ThreadCheck._fields_[1][1](lambda _host: threading.get_ident() == self.audio_ident))
        self.extension = Host._fields_[6][1](
            lambda _host, name: C.addressof(self.thread_check) if name == b"clap.thread-check" else None)
        self.restart = Host._fields_[7][1](lambda _host: None)
        self.request_process = Host._fields_[8][1](lambda _host: None)
        self.callback = Host._fields_[9][1](self.request_callback)
        self.host = Host(Version(1, 2, 2), None, b"ShapeRack validation host", b"ShapeRack",
                         b"https://github.com/0xPRISMM/shaperack", b"1.0",
                         self.extension, self.restart, self.request_process, self.callback)
        self.library = C.CDLL(str(path))
        self.entry = Entry.in_dll(self.library, "clap_entry")
        self.plugin = None
        self.entry_ready = False
        self.active = False
        self.processing = False
        self.position = 0
        self.sample_rate = 48000.0
        require(self.entry.version.major == 1, "Incompatible CLAP entry")
        require(self.entry.init(str(path).encode("utf-8")), "CLAP entry init failed")
        self.entry_ready = True
        factory_ptr = self.entry.get_factory(b"clap.plugin-factory")
        require(factory_ptr, "No CLAP plugin factory")
        factory = C.cast(factory_ptr, FP)
        require(factory.contents.get_plugin_count(factory) == 1, "Expected one plugin")
        descriptor_ptr = factory.contents.get_plugin_descriptor(factory, 0)
        require(descriptor_ptr, "Missing descriptor")
        descriptor = descriptor_ptr.contents
        require(descriptor.name == b"ShapeRack", f"Unexpected plugin: {descriptor.name!r}")
        require(descriptor.id == b"audio.prismm.shaperack", "Unexpected plugin ID")
        self.plugin = factory.contents.create_plugin(factory, C.byref(self.host), descriptor.id)
        require(self.plugin, "Plugin creation failed")
        require(self.plugin.contents.init(self.plugin), "Plugin init failed")
        self.params = self.extension_for(b"clap.params", Params)
        self.state = self.extension_for(b"clap.state", State)
        self.infos = {}
        for index in range(self.params.count(self.plugin)):
            info = ParamInfo()
            require(self.params.get_info(self.plugin, index, C.byref(info)), "get_info failed")
            name = info.name.decode("utf-8")
            require(name not in self.infos, f"Duplicate parameter name {name}")
            self.infos[name] = info
            require(info.min_value <= self.value(name) <= info.max_value,
                    f"Parameter outside its declared range: {name}")
            text = C.create_string_buffer(256)
            require(self.params.value_to_text(self.plugin, info.id, info.default_value,
                                              text, len(text)), f"No parameter text: {name}")
        required = {"Mix", "Bypass", "Volume point 00", "Volume enabled", "Filter enabled",
                    "Pan enabled", "Drive enabled", "Cycle length", "Tempo sync"}
        require(required.issubset(self.infos), f"Missing parameters: {required - self.infos.keys()}")
        require(len(self.infos) >= 80, "Curve points were not exported as host parameters")
        require(self.infos["Bypass"].flags & (1 << 4), "Bypass is not identified to the host")
        print(f"PASS factory/descriptor/init; {len(self.infos)} host parameters")

    def request_callback(self, _host):
        self.callback_requested = True

    def service_callbacks(self):
        if self.callback_requested and self.plugin:
            self.callback_requested = False
            self.plugin.contents.on_main_thread(self.plugin)

    def extension_for(self, name, structure):
        extension = self.plugin.contents.get_extension(self.plugin, name)
        require(extension, f"Missing {name.decode()} extension")
        return C.cast(extension, C.POINTER(structure)).contents

    def value(self, name):
        value = F64()
        require(self.params.get_value(self.plugin, self.infos[name].id, C.byref(value)),
                f"get_value failed: {name}")
        return value.value

    def event(self, name, value, time=0):
        info = self.infos[name]
        require(info.min_value <= value <= info.max_value, f"Invalid value for {name}")
        return ParamEvent(EventHeader(C.sizeof(ParamEvent), time, 0, 5, 0),
                          info.id, info.cookie, -1, -1, -1, -1, value)

    def set(self, updates):
        require(not self.active, "Inactive parameter flush must run before activation")
        events = EventList(self.event(name, value) for name, value in updates.items())
        self.params.flush(self.plugin, C.byref(events.input), C.byref(events.output))
        self.service_callbacks()
        for name, value in updates.items():
            require(abs(self.value(name) - value) < 1e-6, f"Parameter flush failed: {name}")

    def activate(self, sample_rate=48000.0):
        self.sample_rate = sample_rate
        self.position = 0
        require(self.plugin.contents.activate(self.plugin, sample_rate, 1, 512), "Activation failed")
        self.active = True
        self.executor.submit(self.plugin.contents.reset, self.plugin).result()
        require(self.executor.submit(self.plugin.contents.start_processing, self.plugin).result(),
                "start_processing failed")
        self.processing = True

    def deactivate(self):
        if self.processing:
            self.executor.submit(self.plugin.contents.stop_processing, self.plugin).result()
            self.processing = False
        if self.active:
            self.plugin.contents.deactivate(self.plugin)
            self.active = False
        self.service_callbacks()

    def block(self, count=512, tempo=120.0, events=(), beat=None):
        require(self.processing and 1 <= count <= 512, "Invalid process call")
        inputs = [(C.c_float * count)(*([value] * count)) for value in (0.25, 0.15)]
        outputs = [(C.c_float * count)() for _ in range(2)]
        input_ptrs = (C.POINTER(C.c_float) * 2)(*inputs)
        output_ptrs = (C.POINTER(C.c_float) * 2)(*outputs)
        in_buffer = AudioBuffer(input_ptrs, None, 2, 0, 0)
        out_buffer = AudioBuffer(output_ptrs, None, 2, 0, 0)
        seconds = self.position / self.sample_rate
        beat = seconds * tempo / 60.0 if beat is None else beat
        transport = Transport()
        transport.header = EventHeader(C.sizeof(Transport), 0, 0, 9, 0)
        transport.flags = 1 | 2 | 4 | 8 | 16  # tempo, timelines, time signature, playing
        transport.song_pos_beats = round(beat * (1 << 31))
        transport.song_pos_seconds = round(seconds * (1 << 31))
        transport.tempo = tempo
        transport.tsig_num, transport.tsig_denom = 4, 4
        event_list = EventList(events)
        process = Process(self.position, count, C.pointer(transport), C.pointer(in_buffer),
                          C.pointer(out_buffer), 1, 1,
                          C.pointer(event_list.input), C.pointer(event_list.output))
        status = self.executor.submit(self.plugin.contents.process, self.plugin, C.byref(process)).result()
        require(status in (1, 2, 3, 4), f"Invalid process status {status}")
        rendered = [list(channel) for channel in outputs]
        require(all(math.isfinite(value) for channel in rendered for value in channel),
                "Non-finite output")
        self.position += count
        self.service_callbacks()
        return rendered

    def save(self):
        require(not self.active, "Save this test state while inactive")
        payload = bytearray()

        def write(_stream, buffer, count):
            count = min(count, 37)  # Exercise the plugin's short-write handling.
            payload.extend(C.string_at(buffer, count))
            return count

        callback = OutputStream._fields_[1][1](write)
        stream = OutputStream(None, callback)
        require(self.state.save(self.plugin, C.byref(stream)), "State save failed")
        require(len(payload) > 64, "Saved state is unexpectedly empty")
        return bytes(payload)

    def load(self, payload):
        require(not self.active, "Load this test state while inactive")
        position = 0

        def read(_stream, buffer, count):
            nonlocal position
            count = min(count, 29, len(payload) - position)
            if count:
                C.memmove(buffer, payload[position:position + count], count)
                position += count
            return count

        callback = InputStream._fields_[1][1](read)
        stream = InputStream(None, callback)
        require(self.state.load(self.plugin, C.byref(stream)), "State load failed")
        self.service_callbacks()

    def close(self):
        self.deactivate()
        if self.plugin:
            self.plugin.contents.destroy(self.plugin)
            self.plugin = None
        if self.entry_ready:
            self.entry.deinit()
            self.entry_ready = False
        self.executor.shutdown(wait=True)


def assert_dry(channels):
    for channel, expected in zip(channels, (0.25, 0.15)):
        require(max(abs(value - expected) for value in channel) < 1e-6,
                "Mix zero or bypass did not produce dry audio after settling")


def validate(host):
    host.activate()
    output = []
    for _ in range(100):
        output.extend(host.block()[0])
    require(max(output) - min(output) > 0.12, "Default volume curve does not pump")
    require(min(output) < 0.05 and max(output) > 0.20, "Default pump has an unexpected range")
    print(f"PASS default pump: gain {min(output) / 0.25:.4f}..{max(output) / 0.25:.4f}")
    host.deactivate()

    for tempo, sample_rate in ((83.0, 48000.0), (128.0, 44100.0), (173.0, 96000.0)):
        host.activate(sample_rate)
        for count in (1, 17, 64, 127, 512):
            host.block(count, tempo)
        # A seek to opposite parts of the beat should follow the transport curve.
        low = high = None
        for _ in range(10):
            low = host.block(512, tempo, beat=0.0)[0][-1]
        for _ in range(10):
            high = host.block(512, tempo, beat=0.80)[0][-1]
        require(high - low > 0.10, f"Transport seek did not realign at {tempo} BPM")
        host.deactivate()
    print("PASS variable buffers, 44.1/48/96 kHz, 83/128/173 BPM, transport seeks")

    host.activate()
    reference = host.block(512)
    host.deactivate()
    host.activate()
    first = host.block(512, events=[host.event("Mix", 0.0, time=128)])
    require(max(abs(actual - expected) for actual, expected
                in zip(first[0][:128], reference[0][:128])) < 1e-6,
            "Future automation event affected samples before its timestamp")
    require(first[0][-1] > reference[0][-1] + 0.10,
            "Timestamped Mix automation did not affect subsequent samples")
    for _ in range(8):
        settled = host.block()
    assert_dry(settled)
    require(abs(host.value("Mix")) < 1e-6, "Process automation did not update Mix")
    host.deactivate()
    host.set({"Mix": 1.0})
    host.activate()
    multiple = host.block(512, events=[host.event("Mix", 0.0, time=128),
                                      host.event("Mix", 1.0, time=384)])
    require(max(abs(actual - expected) for actual, expected
                in zip(multiple[0][:384], first[0][:384])) < 1e-6,
            "A later automation event affected an earlier sub-block")
    require(multiple[0][-1] < first[0][-1] - 0.01,
            "Multiple automation events did not advance correctly")
    require(abs(host.value("Mix") - 1.0) < 1e-6, "Last automation event was not applied")
    host.deactivate()
    host.activate()
    host.block(events=[host.event("Bypass", 1.0, time=64)])
    for _ in range(10):
        settled = host.block()
    assert_dry(settled)
    host.deactivate()
    print("PASS timestamped process automation; mix zero and bypass pass dry audio")

    host.set({"Bypass": 0.0, "Mix": 0.37, "Volume point 00": 0.17,
              "Filter enabled": 1.0, "Pan enabled": 1.0, "Drive enabled": 1.0})
    snapshot = {name: host.value(name) for name in host.infos}
    payload = host.save()
    host.set({"Mix": 0.91, "Volume point 00": 0.83})
    require(host.save() != payload, "Edited parameter state did not change")
    host.load(payload)
    require(all(abs(host.value(name) - value) < 1e-6 for name, value in snapshot.items()),
            "State roundtrip lost a parameter or edited curve point")
    host.activate()
    for _ in range(10):
        host.block()
    host.deactivate()
    print(f"PASS state save/load ({len(payload)} bytes), all parameter values and edited curve")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plugin", type=Path, help="Path to the ShapeRack CLAP binary")
    args = parser.parse_args()
    check_abi()
    path = args.plugin.resolve(strict=True)
    host = TestHost(path)
    try:
        validate(host)
    finally:
        host.close()
    print("ShapeRack CLAP integration validation passed.")


if __name__ == "__main__":
    main()
