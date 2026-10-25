# dubdsr

Empilement GPU Soleil / Lune pour les vidéos RAW SeeStar. Le programme lancé est un binaire Rust pur.

La seule pièce à avoir sur la machine est le pilote NVIDIA déjà installé (`nvcuda.dll` sous Windows, `libcuda.so.1` sous Ubuntu).

## Lancer

Sous Windows, double-cliquez `dubdsr.exe`. Sous Ubuntu, lancez `./dubdsr`.

La fenêtre s’ouvre même si la carte NVIDIA n’est pas détectée : un avertissement l’indique, et l’empilement échouera tant que le pilote n’est pas disponible. Une vidéo trop longue pour la mémoire de la carte échoue avec un message explicite.

Langues : français, anglais, espagnol, allemand. `Ctrl+L` les fait défiler. La langue choisie est enregistrée avec les réglages Soleil et Lune, puis réappliquée au lancement :

- Windows : `%APPDATA%\dubdsr\settings.json`
- Ubuntu : `$XDG_CONFIG_HOME/dubdsr/settings.json`, ou `~/.config/dubdsr/settings.json`

Le TIFF 16 bits `{nom}_stacked.tiff` est écrit à côté de la vidéo source.

## Compiler

Le compilateur de nuit et la cible GPU sont nécessaires pour produire le binaire. L’utilisateur du `.exe` n’en a pas besoin.

```bash
rustup toolchain install nightly
rustup target add nvptx64-nvidia-cuda --toolchain nightly
cargo build --release
```

- Windows : `target/release/dubdsr.exe`. Le runtime C est lié en statique, donc pas de redistribuable Visual C++ à installer.
- Ubuntu : `target/release/dubdsr`. Le binaire s’appuie sur la glibc et les bibliothèques d’un bureau Ubuntu déjà installé, plus le pilote NVIDIA.
