# METRUM RISE

## Description
Open Source (GPLv2) city building game build with Godot game engine and Rust programming language.
License: [GNU General Public License, version 2](https://www.gnu.org/licenses/old-licenses/gpl-2.0.en.html)

## Requirements
- Godot (4.x)
- Rust  (made with 1.97)
- Linux **OR** Windows 10+

**Minimum specs:**
- Not entirely sure. It is however worth noting that I got this running on a *very* cursed
AMD laptop from 2017 just to see, and it ran the intersection spike without melting
at least.


## How-to

To start the game, run the following command in terminal:
```
./run.sh --release
```

For asset editor
```
./run.sh --release --asset-editor
```

There are also economy and world editor. But these are not update in awhile
so they are not that usable currently.


## Bugs
Loads of bugs: less of the old ones, probably more new ones.

## How I got here - BantedHam

I happened upon this project by way of [Easily Distracted Games video](https://youtu.be/maKrOClAxgA?t=352) on the
19th of August, 2026, and got excited about two things at once: testing my own
game engine on a new project, and contributing to something I had been circling
for years. The only reason I have a GitHub account now is to work on this. I
have been tossing around ideas for a city builder for a long time and had never
found the right place to put them.

The 2.5D_engine run on Godot, is written in C++ and gdscript, and is internal.
It is not released and there is no public repository for it. It is a Pixel art
engine, but that was a choice made necessary by trying to run unreasonable
simulations on decrepit hardware, not a hard constraint. One of its strenghts
is exactly what a city builder needs, to run very large numbers of individual
agents, and deriving physical behavior rather than scripting it. Water, fire,
wind, minerals, and the rest of the physical layer come out of general laws
reading shared fields, so a fire reads the weather and the fuel underneath it
instead of consulting a table.

Then I opened the source and found the whole thing is written in Rust...

After poking around for a few hours, I got about 50 lines into the rewrite
before I had the sobering realisation that I really do not want to do this
right now. So I poked around some more instead, and landed on getting windows
actually running (in hindsight, its hard to understand why that wasn't my
first instinct), flushing out the docs and working on roads as my first push.

Porting the parts of my engine this game needs is going to look like me
running some sort of agentic harness for several days and then fixing it for
a few weeks after, because I refuse to rewrite 44,000 lines of code by hand
in a language I am not very familiar with for an open source project. 

The target is now 20,000,000 individual agents across an entire country,
possibly even in a single city, though nobody has volunteered to melt their
computer to find out yet, and my current daily driver doesnt even have a
dedicated GPU (compute shaders for the win).

## What has been done so far

Contributions to this repository, in the order they landed:
- Lanes have an identity and are no longer counted. An edge used to describe
  its lanes as two integers, this cannot express a median, a bus lane, a cycle
  track, curbside parking, a turn pocket, or lanes of varying sizes. A road is
  now an ordered cross-section of bands with its own kind, direction, width,
  permitted modes, marking, turn set, and longitudinal range.
- Asymmetric and reversible roads. Three lanes with a shared center, tidal
  lanes that flip direction without moving, and turn pockets that widen an
  approach.
- Border policy is continuous. The migration term that was binary, either a
  border existed or it didn't, is now a dial with four states derived from it.
- Frontage roles. Building frontage states what it is for so a service way
  refuses to carry an address. Without that rule an alley is a thin street and
  the allocator fills it with houses facing the wrong way.
- A Windows build. `run.ps1` mirrors `run.sh` flag for flag. The crate
  builds for `x86_64-pc-windows-msvc`, the resulting DLL loads in Godot, and a
  headless run exits cleanly. You are right, Linux is better. I however happen
  to be working off of Windows currently.
- Documentation. `docs/narrative.md`, `docs/region.md`, and
  `docs/services.md` are new; world generation and minerals went into
  `docs/terrain.md`, and the lane model into `docs/roads.md`.
- Junction control. A node carries priority signs, main, yield, or
  stop per arm, or a timed signal program with green, amber, and a cycle offset
  so junctions along a corridor can be progressed into a green wave. Reachable
  from GDScript, though no tool drives it yet.
- Conflicting movements. Each turn through a junction is its own
  connector lane, and a lane separates cars only along its own length, so a left
  turn and the oncoming through movement never tested each other. A per-node
  table now records which paths cross. Both directions of one street still run
  together, the way a signal phase works.
- Congestion feeds back into routing. Congestion was already
  measured per tick and already priced into the router, but a car holding a valid
  path never asked again. It now reprices the remainder of its route at each
  junction and switches on a 15% improvement.
- Known broken: right turns still collide. Turning lanes and a turn
  hierarchy are the next thing to build, and the conflict table cannot resolve the
  better claim without them.

## Credits

### Kuopio map:
Credits: National Land Survey of Finland/Heighmap of Kuopio/the National Land Survey
of Finland Topographic Database/ Date: 19.04.2026)

### 3rd party assets used
- https://ambientcg.com/view?id=DaySkyHDRI059B
- https://www.cgbookcase.com/textures/grass-01
- https://polyhaven.com/a/concrete_layers_02
- https://polyhaven.com/a/asphalt_04
- https://polyhaven.com/a/clean_asphalt
- https://polyhaven.com/a/withered_grass
- https://polyhaven.com/a/dark_rock

- https://quaternius.com/
- https://kenney.itch.io/


## Screenshots

They are at the "screenshots" folder.
- [Main view](/screenshots/town_01.png)
- [Asset Editor](/screenshots/asset_editor_01.png)
- [First attempt to make a t-junction](/screenshots/t-junction.png)


## Video
45 min video about this buggy game on how it looks on release
- [Metrum Rise - Open Source City Building Game](https://youtu.be/QtjniXLWW9M)
