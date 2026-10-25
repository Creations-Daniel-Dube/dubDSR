[Français](#français) · [English](#english)

# Français

[English](#english)

# dubDSR v1.1.1 aka **dub Disc Stacking Rust**

Empilement Nividia GPU Soleil / Lune pour les vidéos RAW SeeStar et les séries de photos RAW d’appareil photo. Le programme lancé est un binaire Rust pur.

La seule pièce à avoir sur la machine est le pilote NVIDIA déjà installé (`nvcuda.dll` sous Windows, `libcuda.so.1` sous Ubuntu).

## Lancer

Sous Windows, double-cliquez `dubdsr.exe`. Sous Ubuntu, lancez `./dubdsr`.

La fenêtre s’ouvre même si la carte NVIDIA n’est pas détectée : un avertissement l’indique, et l’empilement échouera tant que le pilote n’est pas disponible. Une vidéo trop longue, ou une série RAW trop large pour la mémoire de la carte, échoue avec un message explicite.

Langues : français, anglais, espagnol, allemand. `Ctrl+L` les fait défiler. La langue choisie est enregistrée avec les réglages Soleil et Lune, puis réappliquée au lancement :

- Windows : `%APPDATA%\dubdsr\settings.json`
- Ubuntu : `$XDG_CONFIG_HOME/dubdsr/settings.json`, ou `~/.config/dubdsr/settings.json`

Le bouton **?** à droite du titre de l’aperçu ouvre l’aide, et le survol d’un contrôle affiche le même texte.

Les réglages Soleil et Lune sont les mêmes pour les deux sources : part des images les plus nettes, alignement, recadrage, drizzle, tons. En motif Bayer **Auto**, la vidéo est analysée, et une série RAW reprend le motif lu dans les fichiers.

## Vidéo SeeStar

**Vidéo…** ouvre un AVI. Le TIFF 16 bits `{nom}_stacked.tiff` est écrit à côté de la vidéo.

## Dossier RAW

**Photos…** ouvre un dossier de photos. On peut aussi coller le chemin du dossier dans le champ source. Les fichiers reconnus, triés par nom, sont :

`CR2`, `CR3`, `NEF`, `NRW`, `ORF`, `PEF`, `ARW`, `DNG`, `RAF`, `RW2`

Le décodage (mosaïque Bayer et orientation) est fait par LibRaw, déjà compilé dans le binaire. L’aperçu affiche la miniature embarquée quand le fichier en contient une.

Toute la série doit avoir la même taille et le même motif. Un fichier différent arrête l’empilement en nommant le fichier. Une plage de frames limite la série aux fichiers compris entre les deux index, dans l’ordre des noms.

Le TIFF 16 bits `{dossier}_stacked.tiff` est écrit dans le dossier source.

## Compiler

Le compilateur de nuit, la cible GPU et un compilateur C++ sont nécessaires pour produire le binaire. LibRaw est compilé avec lui. L’utilisateur du `.exe` n’en a pas besoin.

```bash
rustup toolchain install nightly
rustup target add nvptx64-nvidia-cuda --toolchain nightly
cargo build --release
```

- Windows : `target/release/dubdsr.exe`. Le runtime C est lié en statique, donc pas de redistribuable Visual C++ à installer.
- Ubuntu : `target/release/dubdsr`. Le binaire s’appuie sur la glibc et les bibliothèques d’un bureau Ubuntu déjà installé, plus le pilote NVIDIA.

# English

[Français](#français)

# dubDSR v1.1.1 aka dub Disc Stacking Rust

Nvidia GPU stacking of the Sun and Moon for SeeStar RAW videos and camera RAW photo series. The program you run is a pure Rust binary.

The only thing the machine needs is the NVIDIA driver already installed (`nvcuda.dll` on Windows, `libcuda.so.1` on Ubuntu).

## Run

On Windows, double-click `dubdsr.exe`. On Ubuntu, run `./dubdsr`.

The window opens even when no NVIDIA card is detected: a warning says so, and stacking fails until the driver is available. A video that is too long, or a RAW series that is too wide for the card’s memory, fails with an explicit message.

Languages: French, English, Spanish, German. `Ctrl+L` cycles them. The chosen language is saved with the Sun and Moon settings and applied again at launch:

- Windows: `%APPDATA%\dubdsr\settings.json`
- Ubuntu: `$XDG_CONFIG_HOME/dubdsr/settings.json`, or `~/.config/dubdsr/settings.json`

The **?** button to the right of the preview title opens the help, and hovering a control shows the same text.

Sun and Moon settings are the same for both sources: share of the sharpest frames, alignment, crop, drizzle, tone. With Bayer pattern **Auto**, the video is analyzed, and a RAW series uses the pattern read from the files.

## SeeStar video

**Video…** opens an AVI. The 16-bit TIFF `{name}_stacked.tiff` is written next to the video.

## RAW folder

**Photos…** opens a folder of photos. The folder path can also be pasted into the source field. Recognized files, sorted by name, are:

`CR2`, `CR3`, `NEF`, `NRW`, `ORF`, `PEF`, `ARW`, `DNG`, `RAF`, `RW2`

Decoding (Bayer mosaic and orientation) is done by LibRaw, already compiled into the binary. The preview shows the embedded thumbnail when the file has one.

The whole series must share the same size and the same pattern. A different file stops stacking and names that file. A frame range limits the series to the files between the two indexes, in name order.

The 16-bit TIFF `{folder}_stacked.tiff` is written inside the source folder.

## Build

A nightly compiler, the GPU target, and a C++ compiler are required to produce the binary. LibRaw is compiled with it. The user of the `.exe` does not need them.

```bash
rustup toolchain install nightly
rustup target add nvptx64-nvidia-cuda --toolchain nightly
cargo build --release
```

- Windows: `target/release/dubdsr.exe`. The C runtime is linked statically, so no Visual C++ redistributable has to be installed.
- Ubuntu: `target/release/dubdsr`. The binary relies on the glibc and libraries of an Ubuntu desktop that is already installed, plus the NVIDIA driver.
