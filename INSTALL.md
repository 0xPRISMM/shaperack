# ShapeRack — Windows x64 installation

1. Extract the release ZIP.
2. Copy the **entire `ShapeRack.vst3` folder**, including `Contents`, to:

   `C:\Program Files\Common Files\VST3\ShapeRack.vst3`

   Windows may request administrator approval when you copy into Program Files.
3. In FL Studio, open **Options → Manage plugins**, enable **Verify plugins**, and run **Find installed plugins**.
4. Open a mixer insert effect slot and select **ShapeRack** from the plugin list. Favorite it in Plugin Manager if desired.

These locations and scanning steps follow the [official FL Studio installation guide](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/basics_externalplugins.htm).

For a host supporting CLAP, copy `ShapeRack.clap` to its CLAP search folder (typically `C:\Program Files\Common Files\CLAP`) and rescan. VST3 is the primary FL Studio package.

ShapeRack starts with a quarter-note volume pump enabled. Reduce Volume Amount or Mix for a softer effect. The Filter, Pan, and Drive modules start disabled.

Version 0.1.0 is an experimental first release; see README and VALIDATION for the implemented features and checks. It requires a 64-bit host and a graphics driver supporting the editor's OpenGL renderer.

To uninstall, remove the installed ShapeRack VST3 folder and/or CLAP file and rescan the host. Existing projects using ShapeRack will then report a missing plugin.
