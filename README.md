# dubdsr

Empilement GPU Soleil / Lune pour les vidéos RAW SeeStar et les séries de photos RAW d’appareil photo. Le programme lancé est un binaire Rust pur.

La seule pièce à avoir sur la machine est le pilote NVIDIA déjà installé (`nvcuda.dll` sous Windows, `libcuda.so.1` sous Ubuntu).

## Lancer

Sous Windows, double-cliquez `dubdsr.exe`. Sous Ubuntu, lancez `./dubdsr`.

La fenêtre s’ouvre même si la carte NVIDIA n’est pas détectée : un avertissement l’indique, et l’empilement échouera tant que le pilote n’est pas disponible. Une vidéo trop longue, ou une série RAW trop large pour la mémoire de la carte, échoue avec un message explicite.

Langues : français, anglais, espagnol, allemand. `Ctrl+L` les fait défiler. La langue choisie est enregistrée avec les réglages Soleil et Lune, puis réappliquée au lancement :

- Windows : `%APPDATA%\dubdsr\settings.json`
- Ubuntu : `$XDG_CONFIG_HOME/dubdsr/settings.json`, ou `~/.config/dubdsr/settings.json`

Les réglages Soleil et Lune sont les mêmes pour les deux sources : part des images les plus nettes, alignement, recadrage, drizzle, tons. En motif Bayer **Auto**, la vidéo est analysée, et une série RAW reprend le motif lu dans les fichiers.

## Vidéo SeeStar

**Parcourir…** ouvre un AVI. Le TIFF 16 bits `{nom}_stacked.tiff` est écrit à côté de la vidéo.

## Dossier RAW

**Dossier…** ouvre un dossier de photos. On peut aussi coller le chemin du dossier dans le champ source. Les fichiers reconnus, triés par nom, sont :

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
