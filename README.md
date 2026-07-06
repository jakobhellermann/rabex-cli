# rabex

Inspect Unity serialized files, asset bundles and addressables from the command line.

```sh
cargo install --git https://github.com/jakobhellermann/rabex-cli
```

## Game selection

Every command needs a game. Pass `--steam-game <name|appid>` or `--game-dir <dir>`, or run
from inside a game directory to auto-detect it.

```sh
rabex --steam-game 'Hollow Knight' game info
rabex --game-dir ~/games/MyGame/MyGame_Data scenes
rabex game info                       # auto-detected from the current directory
```

## Listing things

```sh
rabex game info                       # unity version, file/addressable counts
rabex scenes                          # scenes (build settings + addressables)
rabex files                           # serialized files
rabex bundles                         # asset bundles
rabex addressables                    # addressables keys with asset types
rabex addressables stats              # catalog breakdown by provider/type
rabex game script-locations Player    # scripts (filtered) and where they're defined
```

## Inspecting a serialized file

Reachable via `scene <name>`, `file <path>`, `bundle <path> file [cab]`, or
`addressable <key> file`. All share the same verbs:

```sh
rabex scene Menu_Title info                 # header: version, types, counts
rabex scene Menu_Title tree                 # transform hierarchy
rabex scene Menu_Title tree --components    # + components per GameObject
rabex file globalgamemanagers objects       # path_id  ClassId
rabex scene Crossroads_03 find PlayMakerFSM # GameObjects carrying that component/script
```

## Inspecting one object

An object is selected by path id, `m_Name`, a singleton class name, or a component path:

```sh
rabex scene Menu_Title object -8333123456789 cat                   # dump as JSON, by path id
rabex scene Tutorial_01 object '_Enemies/Crawler 3@DamageHero' cat # by component path
rabex file globalgamemanagers.assets object PlayMakerFSM info
rabex file globalgamemanagers object TagManager info               # singleton by class name
```

## Bundles

The bundle path is resolved relative to the game's addressables build folder (with a game
context), or as a plain filesystem path for a standalone bundle.

```sh
rabex bundle path/to/standalone.bundle files                        # filesystem path
rabex bundle heroloading_assets_all.bundle files                    # addressables-relative
rabex bundle heroloading_assets_all.bundle file CAB-... objects     # inspect a CAB
rabex bundle heroloading_assets_all.bundle file objects             # shortcut to the main CAB
rabex bundle scenes_scenes_scenes/cog_07.bundle file objects        
```

## Addressables

```sh
rabex addressables                    # list all
rabex addressable _GameCameras info   # metadata
rabex addressable _GameCameras file   # go the the defining file
rabex addressable _GameCameras cat    # dump the object data
```

## Finding references

`references` finds every object (across all files) that points at a target.

```sh
rabex file globalgamemanagers object.assets PlayMakerFSM references # who references this object
rabex file globalgamemanagers object.assets PlayMakerFSM references --files-with-matches
rabex bundle .._monoscripts.bundle file object PlayMakerFSM references
rabex bundle .._monoscripts.bundle file object PlayMakerFSM references --exclude scenes
rabex bundle heroloading_assets_all.bundle file object Hero_Hornet@Transform references --exclude-type NailSlashTerrainThunk
```

## Autocomplete

Dynamic completion (completes scene names, object paths, bundles, addressable keys, …) is
built in. Source it for your shell:

```sh
COMPLETE=fish rabex | source                   # fish
source <(COMPLETE=bash rabex)                  # bash
source <(COMPLETE=zsh rabex)                   # zsh
COMPLETE=powershell rabex | Invoke-Expression  # powershell
```

## `rabex --help`

```
Inspect Unity serialized files, asset bundles and game directories.

A game context is set with `--steam-game`/`--game-dir` before the verb, or detected from the
current directory. Plurals list a collection (`scenes`, `bundles`); singulars select one item
then operate (`scene <name> tree`, `bundle <path> file <cab> objects`).

Usage: rabex [OPTIONS] <COMMAND>

Commands:
  game          Game summary
  scenes        List scenes (build settings + addressables)
  files         List the game's serialized files
  bundles       List asset bundles
  addressables  List addressables keys
  scene         Inspect one scene by name
  file          Inspect one serialized file by path
  bundle        Inspect one asset bundle by path
  addressable   Inspect one addressables key
  help          Print this message or the help of the given subcommand(s)

Options:
      --steam-game <NAME>  Locate a game by steam name or app id
      --game-dir <DIR>     Path to a unity game directory (its parent or the `*_Data` dir)
      --format <FORMAT>    Output format [default: pretty] [possible values: pretty, json]
      --color <COLOR>      Colorize `pretty` output [default: auto] [possible values: auto,
                           always, never]
  -h, --help               Print help (see more with '--help')
  -V, --version            Print version
```
