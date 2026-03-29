# Doom Engine — MAGI Implementation Design

**Date**: 2026-03-29
**Status**: Final
**Goal**: Faithful recreation of the id Software Doom engine (v1.9) in MAGI, capable of loading and playing DOOM1.WAD (shareware E1M1-E1M9).

## Constraints

- Pure MAGI source — no C/Rust extensions beyond the existing SDL2/PulseAudio FFI
- Loads unmodified DOOM1.WAD shareware (v1.9, 4,196,020 bytes)
- 320x200 software framebuffer, scaled to SDL window
- Targets 35fps (original Doom tic rate) in interpreted mode
- All rendering, physics, AI computed in MAGI

## Architecture

```
doom/
  main.magi        — SDL init, WAD load, game loop, state machine
  wad.magi         — WAD file parser: header, directory, lump extraction
  map.magi         — Level geometry: vertices, linedefs, sidedefs, sectors, segs, subsectors, nodes, blockmap, reject
  bsp.magi         — BSP tree traversal, front-to-back ordering, node/subsector classification
  render.magi      — Column-based renderer: walls, floors, ceilings, sky, sprites, vis planes
  texture.magi     — Patch/texture/flat compositing, texture column cache, lookup tables
  player.magi      — Player state: position, angle, momentum, health, armor, ammo, keys, inventory
  things.magi      — Mobj (map object) spawning, state machine, think dispatch, sector linking
  enemy.magi       — AI: A_Look, A_Chase, A_FaceTarget, A_Attack, sight checks, sound propagation, pain/death
  weapon.magi      — Weapon state machine: ready, raise, lower, fire, flash — all 9 weapons
  hud.magi         — Status bar, automap, intermission/finale screens, menu system, message display
  sound.magi       — Sound effect loading (from WAD), positional audio mixing, MUS→PCM music playback
  doors.magi       — Sector thinkers: doors, lifts, platforms, crushers, stairs, teleporters, scrollers
  math.magi        — Fixed-point 16.16 arithmetic, BAM angles, trig tables (finesine/finetangent), bounding box
  constants.magi   — Doom enums: mobjtype_t, statenum_t, spritenum_t, linespecials, sector types, weapon types
  tables.magi      — Static data: mobjinfo[], states[], sprnames[], ammo counts, weapon info, damage tables
```

## Game Loop

```
main():
  sdl_init("DOOM", 960, 600)       // 3x scale of 320x200
  wad_open("DOOM1.WAD")
  menu_init()
  state = STATE_MENU

  while running:
    start = sdl_ticks()

    // Input
    poll all SDL events → build input command (forwardmove, sidemove, angleturn, buttons)

    // Tic (35hz)
    if elapsed >= 1000/35:
      match state:
        STATE_MENU    → menu_ticker()
        STATE_LEVEL   → game_ticker()    // P_Ticker: move things, run thinkers, update specials
        STATE_INTERMISSION → wi_ticker()
        STATE_FINALE  → f_ticker()

    // Render (every frame)
    match state:
      STATE_MENU    → menu_drawer()
      STATE_LEVEL   → render_player_view() then hud_drawer()
      STATE_INTERMISSION → wi_drawer()
      STATE_FINALE  → f_drawer()

    blit framebuffer to SDL (scale 320x200 → 960x600)
    sdl_present()
    frame pacing via sdl_delay()
```

## WAD Parser (wad.magi)

### Format
```
Header:    4 bytes "IWAD", i32 numlumps, i32 infotableofs
Directory: numlumps × { i32 filepos, i32 size, 8-byte name }
Lumps:     raw data at filepos
```

### API
```
wad_open(path) → wad handle
wad_find(wad, name) → lump index or -1
wad_read(wad, index) → byte array
wad_read_i16(data, offset) → signed 16-bit
wad_read_i32(data, offset) → signed 32-bit
wad_close(wad)
```

### Level Lumps (per ExMy)
After the ExMy marker lump:
- THINGS: spawn points, items, enemies — 10 bytes each (x, y, angle, type, flags)
- LINEDEFS: wall lines — 14 bytes each (v1, v2, flags, special, tag, sidenum[2])
- SIDEDEFS: wall textures — 30 bytes each (xoffset, yoffset, toptexture, bottomtexture, midtexture, sector)
- VERTEXES: coordinates — 4 bytes each (x, y) as i16
- SEGS: BSP segments — 12 bytes each (v1, v2, angle, linedef, side, offset)
- SSECTORS: subsectors — 4 bytes each (numsegs, firstseg)
- NODES: BSP nodes — 28 bytes each (partition line, bbox[2], children[2])
- SECTORS: floor/ceiling — 26 bytes each (floorheight, ceilingheight, floorpic, ceilingpic, lightlevel, special, tag)
- REJECT: visibility matrix — ceil(numsectors^2 / 8) bytes
- BLOCKMAP: collision grid — header + offset table + block lists

## Rendering Pipeline (render.magi)

### BSP Traversal
1. Start at root node
2. Classify player position against partition line
3. Recurse front child first, then back child
4. At subsector leaf: render all segs in the subsector

### Wall Rendering (column-by-column)
For each seg visible in the subsector:
1. Transform seg endpoints to view space
2. Clip against view frustum (screen columns 0-319)
3. For each screen column x in the seg's range:
   - Calculate wall top/bottom in screen Y from sector heights and distance
   - Apply perspective division (1/distance scaling)
   - Determine texture column from seg offset + fractional x position
   - If upper/lower texture (two-sided line): draw upper and lower walls
   - If middle texture (one-sided): draw full wall
   - Mark ceiling and floor spans above/below the wall

### Visplane Rendering (floors/ceilings)
- Each horizontal span at a given height + flat texture + light level = one visplane
- During wall rendering, mark floor/ceiling open ranges per column
- After all walls: for each visplane, render horizontal spans
- Each pixel: calculate texture coordinates via inverse perspective

### Sprite Rendering
- Collect all visible things in rendered subsectors
- Sort by distance (far to near for painter's algorithm — but Doom uses vissprites with column clipping)
- For each sprite: project to screen, clip columns against wall occlusion (stored from wall rendering)
- Draw visible columns using sprite patches from WAD

### Sky Rendering
- Sky is a 256-pixel-tall texture that scrolls with player angle
- Drawn wherever ceiling flat is "F_SKY1"

### Diminished Lighting
- 256 light levels internally, 32 colormaps in COLORMAP lump
- Distance-based light diminishing: farther walls use darker colormaps
- Sector light level determines base brightness
- Flash effects (gunfire) temporarily increase light

### Framebuffer
- `fb[320 * 200]` array of palette indices (0-255)
- PLAYPAL lump: 14 palettes × 256 RGB triples (normal, damage red, pickup gold, radiation suit green, etc.)
- COLORMAP lump: 34 maps × 256 entries (light-to-dark remapping)
- At blit time: map palette indices to RGB, scale 3x, write to SDL pixels

## Texture System (texture.magi)

### Patch Format
- PNAMES lump: list of patch names
- TEXTURE1 lump: texture definitions (name, width, height, patch list with x,y offsets)
- Each patch: column-oriented data with posts (transparency support)
- Compositing: build texture from patches at load time, cache columns

### Flats
- 64x64 raw pixel data, names from F_START to F_END lumps
- Used for floors and ceilings

### Sprite Format
- Lumps between S_START and S_END
- Named SPRTxy where SPRT = 4-char sprite name, x = frame, y = rotation (0 = all angles, 1-8 = directional)
- Same column/post format as patches

## Player (player.magi)

### Movement
- 35 tics/second game clock
- Forward/backward: ±25 units/tic at run, ±12.5 at walk
- Strafing: ±24 units/tic at run, ±10 at walk
- Turning: mouse or keyboard (angleturn mapped to BAM delta)
- Z position: viewheight 41 above floor, smooth step-up for <24 unit stairs
- Momentum: acceleration/friction model (0.90625 friction factor per tic)
- Always-run toggle (shift key)

### Collision
- 16-unit radius bounding circle for player
- Check against blockmap for linedef collisions
- Slide along walls when blocked (P_SlideMove with up to 3 iterations)
- Step up heights <=24 units
- Thing-to-thing collision for pickups, enemies, projectiles

### Use Action
- Fire a 64-unit ray in facing direction
- Activate the nearest special linedef within 64 units
- Triggers door open, switch toggle, lift activate

## Things / Map Objects (things.magi)

### Mobj Structure
Each map object has:
- Position (x, y, z in fixed-point)
- Angle, momentum (momx, momy, momz)
- Health, type (mobjtype_t), flags (MF_SOLID, MF_SHOOTABLE, MF_MISSILE, etc.)
- State machine: current state index, tics remaining
- Sector reference (for height/light), subsector reference
- Linked list membership (sector things, blockmap things)

### State Machine
- `states[]` table: sprite, frame, tics, action function, next state
- Each tic: decrement tics, when 0 → transition to next state, call action
- States: spawn, see, melee, missile, pain, death, xdeath, raise

### Spawning
- Parse THINGS lump: for each entry, look up mobjtype_t by doomednum
- Create mobj at (x, y) with type's spawn state
- Set flags, health, radius, height, speed from mobjinfo[]

## Enemy AI (enemy.magi)

### Action Functions
- **A_Look**: idle scan — check sound target, check sight (use reject table), transition to see state
- **A_Chase**: move toward target — 8-direction movement, try to open doors, random strafe, melee if close, missile if has ranged attack
- **A_FaceTarget**: set angle toward target
- **A_PosAttack / A_SPosAttack / A_CPosAttack**: hitscan attacks (bullet damage)
- **A_TroopAttack**: imp fireball (projectile)
- **A_SargAttack**: demon bite (melee)
- **A_HeadAttack**: cacodemon fireball
- **A_SkullAttack**: lost soul charge
- **A_BruisAttack**: baron fireball
- **A_CyberAttack**: cyberdemon rockets
- **A_SpidAttack**: spider mastermind chaingun
- **A_BspiAttack**: arachnotron plasma
- **A_VileChase / A_VileAttack**: archvile flame + resurrect
- **A_SkelMissile / A_SkelFist**: revenant homing missile + punch
- **A_FatAttack**: mancubus triple fireball spread
- **A_Pain**: pain chance check → enter pain state
- **A_Fall**: set MF_CORPSE, drop to floor
- **A_Scream / A_XScream**: play death sound
- **A_BrainSpit / A_SpawnFly / A_SpawnSound**: Icon of Sin cube spawner

### Sight Checks
- Use reject table first (quick sector-pair rejection)
- Then ray-trace through BSP (P_CheckSight): check subsectors along line for blocking 2-sided lines with height restrictions

### Sound Propagation
- When a sound is made, flood-fill through 2-sided lines that aren't sound-blocking
- Enemies in reached sectors get `soundtarget` set → triggers A_Look → A_Chase

## Weapons (weapon.magi)

### All 9 Weapons
| # | Weapon | Ammo | Damage |
|---|--------|------|--------|
| 1 | Fist | - | 2-20 (×10 berserk) |
| 2 | Pistol | Bullets | 5-15 |
| 3 | Shotgun | Shells | 5-15 × 7 pellets |
| 4 | Chaingun | Bullets | 5-15 × 2 |
| 5 | Rocket Launcher | Rockets | 20-160 + blast |
| 6 | Plasma Rifle | Cells | 5-40 |
| 7 | BFG9000 | 40 Cells | 100-800 + tracers |
| 3 | Super Shotgun | 2 Shells | 5-15 × 20 pellets |
| 1 | Chainsaw | - | 2-20 |

### Weapon States
Each weapon has states: deselect (lower), select (raise), ready (bobbing), fire, flash (muzzle flash)
- Weapon switching: lower current → raise new
- Flash state adds extra light to sector temporarily
- BFG: 40-tic charge, fires plasma ball, on impact traces 40 rays for bonus damage

### Ammo
- 4 types: bullets, shells, rockets, cells
- Backpack doubles max capacity
- Ammo pickups give 1x or 2x depending on skill level

## Sector Specials / Thinkers (doors.magi)

### Door Types
- Normal door: open, wait 150 tics, close
- Blazing door: fast open/close (4x speed)
- Locked doors: require blue/red/yellow key
- Remote doors: triggered by switch/walkover linedef

### Lifts
- Lower to lowest adjacent floor, wait, raise back
- Perpetual lifts (toggle)
- Speed variants

### Platforms / Crushers
- Ceiling crushers: lower ceiling, raise, repeat — damage things caught
- Slow/fast crusher variants
- Stop on trigger

### Stairs
- Build stairs: each sector in sequence raises floor by 8 units
- Turbo stairs: 16 units

### Floor/Ceiling Movers
- Lower floor to lowest adjacent
- Raise floor to highest adjacent
- Lower ceiling
- Raise floor to match ceiling (crush)

### Teleporters
- Walk-over linedef → move player to thing type 14 in target sector
- Preserve angle, set momentums to zero
- Telefrag anything at destination

### Scrolling
- Scrolling wall textures (push/pull)
- Scrolling floors (conveyors)

### Sector Types
- Type 1-17: damage floors (5/10/20 per interval), blinking lights, oscillating lights, secret sector, etc.

### Line Specials
- ~140 linedef special types
- Trigger types: W1 (walk once), WR (walk repeatable), S1 (switch once), SR (switch repeatable), G1 (gun once), GR (gun repeatable)
- Each maps to a thinker function

## Sound (sound.magi)

### Sound Effects
- Lumps DS* in WAD: 8-bit unsigned PCM at 11025 Hz
- Header: format (3), sample rate (11025), num samples, pad
- ~100 sound effects: weapon fire, enemy alert/pain/death, doors, switches, pickups

### Positional Audio
- Distance attenuation (linear falloff, max range ~1200 units)
- Left/right panning based on angle to source
- Up to 8 simultaneous channels
- Priority system: replace lowest-priority sound when full

### Music (MUS Format)
- MUS lumps (D_E1M1, etc.): MIDI-like format
- Parse MUS header, channel map, events
- Convert note-on/off to PCM via simple FM synthesis or wavetable
- Stream to PulseAudio via speaker package

## HUD (hud.magi)

### Status Bar
- Bottom 32 pixels of screen
- STBAR background patch
- Health %, armor %, current ammo, max ammo
- Arms indicators (1-7 weapon ownership)
- Key cards (blue/yellow/red skull/card)
- Animated face (STF*): health-based expressions, look left/right, ouch, rampage, god mode, dead

### Automap
- Toggle with TAB
- Draw all visible linedefs (color-coded: red=1-sided, yellow=2-sided with height change, brown=same-height, etc.)
- Player arrow, thing triangles
- Pan with arrow keys, zoom with +/-
- Grid toggle, mark spots, follow mode

### Messages
- Pickup messages ("Picked up a shotgun!")
- 4-second display with fade

### Menus
- Main menu: New Game, Options, Load, Save, Quit
- Episode select, skill select
- Options: mouse sensitivity, SFX/music volume, screen size, messages on/off
- Save/Load: 6 slots with 24-char names
- Pause screen

### Intermission
- After completing a level: kills %, items %, secrets %, time, par time
- Animated background, "pistol start" or carry-over stats
- "Entering ExMy" transition screen

### Finale
- E1M8 victory text, cast of characters sequence (E3M8)
- Bunny scroll (E3 ending)

## Math (math.magi)

### Fixed-Point 16.16
- All game positions/distances in fixed-point: integer part (16 bits) + fractional part (16 bits)
- Multiplication: `(a * b) >> 16`
- Division: `(a << 16) / b`
- Conversion: `int_to_fixed(n) = n << 16`, `fixed_to_int(f) = f >> 16`

### Angles (BAM — Binary Angle Measurement)
- Full circle = 0x100000000 (4294967296)
- 90 degrees = 0x40000000 (ANG90)
- Angle stored as u32, wraps naturally
- Fine angle = angle >> ANGLETOFINESHIFT (19)

### Trig Tables
- FINESINE: 10240 entries (8192 sine + 2048 for cosine overlap)
- FINETANGENT: 4096 entries
- Precomputed at init from floating-point, stored as fixed-point

### Point/Line Math
- `point_on_side(x, y, node)` — which side of BSP partition
- `point_to_angle(x1, y1, x2, y2)` — BAM angle between two points
- `point_to_dist(x1, y1, x2, y2)` — approximate distance
- `line_opening(linedef)` — gap between floor/ceiling for two-sided lines
- `box_on_line_side(bbox, partition)` — bounding box vs partition line
- `intersect(x1,y1,x2,y2,x3,y3,x4,y4)` — line-line intersection

## Constants & Tables (constants.magi, tables.magi)

### mobjinfo[] — 137 entries
Each enemy/item/projectile type: doomednum, spawnstate, spawnhealth, seestate, seesound, reactiontime, attacksound, painstate, painchance, painsound, meleestate, missilestate, deathstate, xdeathstate, deathsound, speed, radius, height, mass, damage, activesound, flags, raisestate

### states[] — 967 entries
Each state: sprite, frame, tics, action, nextstate

### sprnames[] — 138 sprite name strings

### Linedef specials — ~140 types mapped to handler functions

### Sector types — 17 types with damage/light behavior

## File Sizes (Estimated)

| File | Lines | Description |
|------|-------|-------------|
| main.magi | 400 | Game loop, state machine, SDL interface |
| wad.magi | 500 | WAD parser, lump extraction |
| map.magi | 600 | Level data loading, structure definitions |
| bsp.magi | 400 | BSP traversal |
| render.magi | 2,500 | Wall/floor/ceiling/sprite rendering, visplanes, clipping |
| texture.magi | 800 | Texture compositing, column cache, flat loading |
| player.magi | 800 | Movement, collision, use action, inventory |
| things.magi | 600 | Mobj management, state machine, sector linking |
| enemy.magi | 1,500 | All AI action functions, sight checks, sound propagation |
| weapon.magi | 800 | Weapon state machines, attack functions, ammo |
| hud.magi | 1,500 | Status bar, automap, menus, intermission, finale |
| sound.magi | 600 | SFX loading/mixing, MUS playback |
| doors.magi | 1,200 | All sector specials, thinkers, line special dispatch |
| math.magi | 300 | Fixed-point, trig, geometry |
| constants.magi | 500 | Enums, flags, doomednums |
| tables.magi | 3,000 | mobjinfo[], states[], sprnames[] (data tables) |
| **Total** | **~16,000** | |

## Input Mapping

| Key | Action |
|-----|--------|
| W / Up | Move forward |
| S / Down | Move backward |
| A | Strafe left |
| D | Strafe right |
| Left | Turn left |
| Right | Turn right |
| Space | Use / Open |
| Ctrl / LClick | Fire |
| Shift | Run |
| 1-7 | Select weapon |
| Tab | Toggle automap |
| Escape | Menu |
| +/- | Automap zoom |
| F1-F6 | Quicksave/load, help |

## Success Criteria

1. `magi run doom/main.magi` opens a 960x600 SDL window
2. DOOM1.WAD shareware loads without errors
3. Title screen displays with Doom logo
4. New Game → Episode 1 → Hurt Me Plenty starts E1M1
5. Player can walk through E1M1, open doors, find secrets
6. Enemies wake up, chase, attack — player can fight back with all weapons
7. Picking up items works (health, ammo, armor, keys)
8. Level exit switch triggers intermission screen
9. Sound effects play for all actions
10. Music plays for each level
11. Status bar shows correct stats with animated face
12. All 9 levels of Episode 1 are completable
13. Framerate stays above 20fps in interpreted mode at 320x200
14. Automap works with correct linedef coloring
