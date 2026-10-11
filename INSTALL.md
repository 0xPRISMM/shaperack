# ShapeRack 0.2.1 — Windows x64 installation

## Install with the EXE

1. Download **[ShapeRack-0.2.1-Setup.exe](https://github.com/0xPRISMM/shaperack/releases/download/v0.2.1/ShapeRack-0.2.1-Setup.exe)** from the GitHub release assets.
2. Close FL Studio and any other host using ShapeRack.
3. Run the setup EXE and approve the administrator prompt. The installer is unsigned, so Windows may identify the publisher as unknown.
4. Keep the VST3 component selected for FL Studio. Select the optional CLAP component if you also want that format installed.
5. Finish setup, open FL Studio, and scan for the plugin as described below.

No Rust toolchain or compilation is needed. Setup installs the plugin files in the standard system locations:

| Component | Installed location |
| --- | --- |
| VST3 | `C:\Program Files\Common Files\VST3\ShapeRack.vst3` |
| Optional CLAP | `C:\Program Files\Common Files\CLAP\ShapeRack.clap` |

The setup EXE installs the effect; you open ShapeRack inside a DAW mixer slot to process audio.

## Find ShapeRack in FL Studio

1. Open **Options → File settings → Manage plugins**.
2. Enable **Verify plugins** and run **Find installed plugins**. For an update, also enable **Rescan previously verified plugins**.
3. Open a mixer insert effect slot and select **ShapeRack** from the plugin list. Favorite it in Plugin Manager if desired.

The locations and scan steps follow the [official FL Studio installation guide](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/basics_externalplugins.htm). VST3 is the primary FL Studio package. Playback, stopped note previews, module presets, and multiple sources routed to one insert were checked in FL Studio 2025. Project save/reopen remains untested; see [VALIDATION.md](VALIDATION.md).

ShapeRack starts with a quarter-note volume pump enabled. Reduce Volume Amount or Mix for a softer effect. Filter, Pan, and Drive start disabled. The plugin requires a 64-bit host and a graphics driver supporting its OpenGL editor.

To enable an effect, select its tab and click the button labeled **Filter OFF**, **Pan OFF**, or **Drive OFF**. It lights up and changes to **ON**; click again to switch it off. Tabs also show each module's ON/OFF state.

**Sync** keeps modulation cycling at the project tempo while you audition notes with playback stopped (**SYNC PREVIEW**). Starting playback realigns modulation with the song position (**SYNC PLAYING**). Use **Free** to set an independent speed in Hz.

## Your preset library

Use **RACK PRESET** to select a factory or personal preset. **Save...** asks for a name and saves the full rack to `%APPDATA%\PRISMM\ShapeRack\Presets`. **Import...** loads a shared `.shaperack.json` file; use Save afterward to keep it in your library. **Export...** opens a native save dialog for sharing or backup. **Refresh** picks up presets saved by another plugin instance.

Preset files include all module settings, curve points and bends, and global audio controls. They leave the editor size alone. Bend opens by default; Nodes, Draw, and Line remain available in the Edit dropdown. See the [curve editing guide](README.md#edit-curves) for snapping and undo/redo.

## ZIP alternative

1. Download and extract the Windows x64 ZIP from the [v0.2.1 release](https://github.com/0xPRISMM/shaperack/releases/tag/v0.2.1).
2. Copy the **entire `ShapeRack.vst3` folder**, including `Contents`, to the VST3 location above. Windows may request administrator approval.
3. If needed, copy `ShapeRack.clap` to the CLAP location above.
4. Rescan the host using the steps above.

## Update or uninstall

Close any host using ShapeRack before updating or removing it. Run the new setup EXE to update files in the standard locations, then rescan the host.

For an installation made with the EXE, open **Windows Settings → Apps → Installed apps**, find **ShapeRack**, and choose **Uninstall**. For a manual ZIP installation, remove the ShapeRack VST3 folder and/or CLAP file yourself. Rescan afterward. Projects using a removed plugin will report it as missing.

Your personal preset files are retained during updates and uninstallation. Back up `%APPDATA%\PRISMM\ShapeRack\Presets` to keep a separate copy or use Export for individual presets.

## Build your own installer

The [README build instructions](README.md#build-from-source) cover the Rust toolchain, C++ build tools, and plugin bundle. After building the bundle, install [Inno Setup 6](https://jrsoftware.org/isinfo.php) and run:

```powershell
powershell -NoProfile -File scripts/package.ps1 -Version 0.2.1
powershell -NoProfile -File scripts/build-installer.ps1
```

Use `-InnoCompiler 'C:\path\to\ISCC.exe'` on the second command if the compiler is installed in a custom location. The resulting setup EXE is written to `dist/`.
