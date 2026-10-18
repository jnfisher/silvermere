# T-049 census: what every casting engine does with each ability

This file lists, for every casting engine (plus the per-round and end-of-spell effects), what
each ability in a spell's ten slots does, and the `SpellEffect` variant and silvermere function
that carry it. It was written before any code (T-049) and is kept as the map from abilities to
behaviour. Ability names are the catalogue's `ABL_` names without the prefix.

The engines, by who casts at what (the engine function names are silvermere's):

| Engine | Who casts at what | Rules function |
|---|---|---|
| `cast_no_target` | a player on himself or on an area | [`apply::apply_effect`] (`CastPath::OnSelf`, `CastPath::Area`) |
| `cast_user_target` | a player on a player | [`apply::apply_effect`] (`CastPath::Single`) |
| `cast_monster_target` | a player on a monster | [`apply::apply_effect`] (`Target::Monster`) |
| `cast_item_target` | a player on an item | [`apply::apply_effect`] (`Target::Item`) |
| `monster_cast` | a monster on a player | `apply_monster::apply_monster_cast` |
| `monster_cast_area` | a monster on the room (or itself) | `apply_monster::apply_monster_cast_area` |
| `cast_self` | a monster's "spells after combat" spell on itself | `apply_monster::apply_monster_self_spell` |
| `room_cast_on_user` | a room spell on a player | [`apply::apply_effect`] (`CastPath::Room`) |
| `room_cast_on_monster` | a room spell on a monster | `apply_monster::apply_room_on_monster` |
| per round and on ending (section E) | a spell in a slot | `apply_round`, `apply_termination`, `apply_monster_round`, `apply_monster_termination` |

## Summary and decisions

### The `SpellEffect` vocabulary

Every ability maps to one of these; no engine needed a variant of its own. `from_slot` builds the
variant from `(ability, value)`; the slot's own value replaces the rolled strength when it is not
0, in every engine.

| Variant | Abilities | Notes |
|---|---|---|
| `Damage { ignore_armour: true }` | DAMAGENOARMOUR | instant hp loss; lasting form damages per round |
| `Damage { ignore_armour: false }` | DAMAGEARMOUR | the magic-resistance multiplier; never lasting |
| `Heal`, `HealMana` | HEAL, HEALMANA | capped at the maximum |
| `Poison`, `CurePoison` | POISON, CUREPOISON | poison level raised to / lowered by the amount |
| `StealHp` | STEALHP | target loses, caster gains (capped) |
| `Energy`, `Hunger`, `Thirst` | ENERGY, HUNGER, THIRST | the field changes by the amount |
| `Summon` | SUMMON | the value is a known-monster number; the engine generates it |
| `Teleport { room }`, `Teleport { map }` | TELEPORTROOM, TELEPORTMAP | both slots needed; the engine moves |
| `ClearItem`, `Scatter`, `TextBlock` | CLEARITEM, SCATTER, TEXTBLOCK | the value is an item, room or text-block number; the engine acts |
| `Shatter`, `RemoveCurse`, `Freedom`, `DispelMagic` | SHATTER, REMOVECURSE, FREEDOM, DISPELMAGIC | act on the target's weapon, worn items or active spells |
| `Slay` | SLAY | hp to one below death (player) or -2 (monster) |
| `StatAlter(stat)`, `AlterHp` | INTELLECT..CHARM, ALTERHP | lasting only; the add decides whether the stat is raised |
| `GiveSpell` | GIVESPELL | lasting only; the value is a spell number |
| `Enslave` | ENSLAVE | monster targets only; charm resistance replaces magic resistance |
| `EndCast`, `EndCastChance` | ENDCAST, ENDCASTCHANCE | the chain after an instant cast; on ending for a lasting one |
| `OpenDoor`, `Lore`, `DetectMagic` | OPENDOOR, LORE, DETECTMAGIC | an `Outcome` for the engine (door chance, hidden descriptions, magic band) |
| `KillDead` | KILLDEAD | a gate on monster targets (undead only); a marker elsewhere |
| `RemoveSpell { chain }` | REMOVESPELL (`chain: true`), TERMSPELL (`chain: false`) | the player engines end the named spell on the target before the other slots; a marker in the room engines and on monsters |
| `Component`, `Guarded` | COMPONENT, GUARDED | prelude: a material component is used up; a guarded monster pulls its guards |
| `Marker(ability)` | NONE, EVILINCOMBAT, AFFECTSANIMAL, QUALITY, GOOD, EVIL, CONFUSEMSG, AFFECTSLIVING, NONLIVING, NOTGOOD, NOTEVIL, NEUTRAL, NOTNEUTRAL, AUTOUSE, DESCMSG, STARTMSG, NONMAGICAL | the "no effect" lists; some are gates in the prelude (noted per engine). DETECTMAGIC is `DetectMagic` (an item effect, nothing on the living) |
| `Lasting(ability)` | every other ability | silly when instant; otherwise a slot entry whose value acts through the ability lookup (T-032) and the per-round code |

### Split decision (card §4)

The monster-side engines differ from the player engines in more than messages for far more than
five abilities: `monster_cast` (DAMAGENOARMOUR and DAMAGEARMOUR without the caster bonus,
DISPELMAGIC ignoring its value, REMOVECURSE without MAJORCURSE, CUREPOISON without the floor,
SUMMON's index range, resist cap 97, slot renewal only when stronger, no duration adjustment),
`monster_cast_area` (DAMAGENOARMOUR hitting nobody for four of the seven area kinds, STEALHP's
gainer, SUMMON without charm, CUREPOISON skipping immune players, fixed slot values ignored by
per-player effects) and `room_cast_on_monster` (charm resistance as the magic resistance, the
linear `150-mr` multiplier capped at `maxval`, HEAL never instant, SUMMON and CUREPOISON
no-ops, every free slot written). The card therefore split:

- **T-049a:** the census, the `SpellEffect` vocabulary, the shared arithmetic (amount, scaling,
  durations, slot tables, target validity, usability), every resistance rule of every engine,
  and `apply` for the player-cast engines and the room-on-player engine, plus the per-round and
  ending effects.
- **T-049b (T-067, `apply_monster.rs`):** `apply` for the monster-cast engines and
  `room_cast_on_monster`, from sections C.2 and D below, reusing the same `SpellEffect` and
  resistance code: `apply_monster_cast` (one slot on one player), `apply_monster_cast_area`
  (one slot on the caster or on every marked player), `apply_monster_self_spell`,
  `apply_room_on_monster` (one slot) and `apply_room_on_monster_spell` (the ten slots, with
  the skip back to slot 6).

### Differences inside the player engines worth tests

DAMAGENOARMOUR scales by the caster's "Alter spell damage" on self and single targets but not in
areas; DAMAGEARMOUR has no lasting form and compounds the spell-damage bonus per monster in an
area; area knock-out tests use the undivided share; POISON and CUREPOISON use the undivided
amount on monsters in an area; monsters never resist the damage abilities in an area but do
resist STEALHP; nothing enforces KILLDEAD in the area loops; the instant area HUNGER and THIRST
do nothing; Full Party Area counts as a self kind for DISPELMAGIC; SUMMON by `cast_no_target`
charms the summon, by `cast_user_target` sets it on the target; SLAY and SHATTER exist only for
a named player.

## A. `cast_no_target`: a player on himself or an area

The engine takes the self kinds (Self, Self or Single, Any: the caster is the target) and the
seven area kinds; any other kind is refused ("You must specify a target"). It runs for a typed
cast, an item or script cast, and a chain cast ("Cast on ending").

### The numbers the effects use

- The strength is uniform in `[min, max]` of [`super::amount_range`] at the caster's level
  (held to the spell's level cap when it has one).
- The per-target share ([`super::per_target_share`]): "Divided" areas and every other kind
  divide the strength by the number of targets (players plus monsters the spell reaches);
  "Full" areas give every target all of it. The caster counts himself for the self kinds,
  Divided Area Incl. You, Full Area, Any and the party kinds; monsters count only for Divided
  Area, Divided Attack, Full Attack, Full Area and Divided Area Incl. You.
- A slot with its own value uses it, undivided, for that slot; only the rolled strength is
  shared. Self effects read the amount; area player loops read the share; area monster loops
  read the share for hit points, energy and heals but the undivided amount for POISON and
  CUREPOISON.
- The element share (attack types only) takes the target's element resistance off the share
  ([`super::resist::element_resistance`]); it is never applied to monsters.
- One success message and one lasting add per cast: once an ability has announced or added the
  spell, later abilities neither announce nor add.
- An evil spell (an attack type cast from an item or a chain, or one with EVILINCOMBAT) skips
  hidden players in every area loop when the caster's evil warnings are on and the only evil
  targets are hidden.
- Area player loops take the players the target count marked (a player who resisted there is
  not marked). Only DAMAGENOARMOUR leaves the caster out unless the kind is Divided Area Incl.
  You or a party kind; the other effects include the caster whenever the kind takes him (Full
  Area, Divided Area Incl. You and the party kinds heal or hit the caster too).
- Area monster loops run only for the five kinds that reach monsters; party kinds never touch
  monsters.
- A monster's resist roll uses its magic resistance (at least 1) with the 98 cap, but only
  STEALHP lets a monster resist; the damage abilities never do (sic).
- A player killed by a spell pays the caster no experience; a player who falls to 0 or below
  from above is knocked out (the area loops test with the undivided share, sic). A monster
  killed pays the caster `exp * level`; a survivor that is not an angel turns on the caster
  with its follow percentage, and an angel already following him stops being friendly.

### After the effects

1. Teleport, when both halves are set and the map is active: Full Party Area moves the caster
   and every group member in the game; any other kind stops the caster following and moves
   him.
2. Without enough energy, mana or level nothing is cast; outside auto-combat the player is told
   which one, in auto-combat the energy is spent if there is enough of it.
3. The secondary stats of every marked player are recalculated.
4. The chain: "Cast on ending" of an instant cast is cast with its chance (default 100), in the
   same mode.
5. The caster's secondary stats are recalculated; in auto-combat the attack goes on (with no
   energy if the mana ran short).

### Abilities

| Ability | Targets | What happens | Variant |
|---|---|---|---|
| COMPONENT | all, before the success roll | the first carried copy of the item with charges loses one; without one, the energy and mana are spent, "You do not have the required material component", the room is told the cast failed, and nothing more. The component is spent even when the cast then fails. | Component |
| EVILINCOMBAT | all, before the success roll | marks the spell evil; outside a safe arena, when the targets include an evil one that is not hidden, the evil warnings run and may stop the cast. It also makes the spell an attack for the protected-room refusal and the player-killing rules. | Marker |
| REMOVESPELL, TERMSPELL | all, before the other slots | the spell number in the slot is ended on every marked player of an area (with its "Cast on ending" for REMOVESPELL, without for TERMSPELL); the player loses his mark and his stats are recalculated. On the self kinds the caster's first matching spell ends and (sic) nothing else of the spell happens. | RemoveSpell |
| TEXTBLOCK | all | the text block runs for the caster; one success message | TextBlock |
| CLEARITEM | all | every copy of the item leaves the caster's room; no message | ClearItem |
| TELEPORTROOM, TELEPORTMAP | all | one half of the destination; the move comes after the effects | Teleport |
| REMOVECURSE | self kinds, instant (areas and lasting silly) | every worn item that is Cursed, or whose Major Curse level is below the caster's level, comes off (forced). The pass over the wielded weapon is broken (sic). | RemoveCurse |
| FREEDOM | self kinds (areas silly) | instant: every active spell with Hold person or Paralyze ends, with its ending; lasting: the spell is added once | Freedom |
| DISPELMAGIC | self kinds and Full Party Area (other areas silly; lasting silly) | every active spell of the caster with the ability the value names (-1: every spell) ends, with its ending; only the caster's, even for Full Party Area | DispelMagic |
| DAMAGENOARMOUR | self kinds; areas | self: instant hp loss scaled by the caster's spell damage bonus, knock-out on the damage; lasting: the spell is added with the unscaled value. Area: each marked player (the caster only for the inclusive kinds) loses his element share with no bonus; each monster loses the share (no element, no bonus, never resisted); lasting: added to the room's targets once. | Damage{ignore_armour:true} |
| DAMAGEARMOUR | self kinds; areas; no lasting form | self: the spell damage bonus added inline, then the player magic-resistance multiplier ([`super::resist::mr_damage`]). Area players: the element share through the same multiplier, with the previous player's cut carried (sic); area monsters: the bonus is added again for every monster, compounding from the last player's share (sic), then the monster multiplier; never resisted. | Damage{ignore_armour:false} |
| ENSLAVE | all | silly: no meaning here | Enslave |
| STEALHP | areas, instant (self kinds and lasting silly) | each marked player loses his element share and the caster gains it, capped at his maximum. Each monster rolls to resist; a resist makes the share 0 for every later monster (sic); otherwise it loses the share and the caster gains it. | StealHp |
| ENERGY | self kinds; areas | self: energy changes by the amount, no message; areas: each marked player gains his element share, each monster the share; lasting: added once | Energy |
| SUMMON | self kinds, instant (lasting and areas silly) | the known monster the amount names is generated in the room, following the caster as a charmed friend | Summon |
| HUNGER, THIRST | self kinds; areas | self: the field changes by the amount; the instant area form does nothing; lasting: added once | Hunger, Thirst |
| HEAL | self kinds; areas | self: hit points rise to at most the maximum (the gain may be negative above it); the message shows the rolled strength. Divided Area with nobody else in the room: "You are the only one in the room!" and nothing happens. Areas: each marked player gains the share up to his maximum, never a negative gain; each monster gains the share up to its maximum. No element share. Lasting: added once. | Heal |
| HEALMANA | self kinds; areas (players only) | mana rises to at most the maximum (self: kept at 0 or above; areas: no floor); the same "only one in the room" rule; lasting: added once | HealMana |
| POISON | self kinds; areas | the immune are skipped; the poison level is raised to the amount if lower (area players: the share; area monsters: the undivided amount), and a monster may turn on the caster; lasting: the raise and the add | Poison |
| CUREPOISON | self kinds; areas | the first poison spell ends ([`apply::terminate_user_poison_spell`]), then the poison falls by the amount, never below 0 for the caster but with no floor in the area loop (sic); on monsters the test is inverted (sic): a level below the undivided amount falls to 0, a level at or above it stays | CurePoison |
| ALTERHP | self kinds, lasting only (instant and areas silly) | the add is made without renewal (an active copy fails the cast: "You attempt to cast ..., but fail."); when it took, or an earlier slot already added the spell, the maximum and current hit points rise by the amount | AlterHp |
| INTELLECT, WISDOM, STRENGTH, HEALTH, AGILITY, CHARM | as ALTERHP | the stat rises by the amount | StatAlter |
| ENDCAST | all | an instant cast remembers the spell to chain; a lasting one chains when it ends | EndCast |
| ENDCASTCHANCE | all | the chain's chance (default 100) | EndCastChance |
| OPENDOOR | all, instant only (lasting silly) | the room's doors and gates are forced with chance `(spellcast - diff)/2`, held to 110, plus each exit's own figure | OpenDoor |
| the markers, KILLDEAD, DETECTMAGIC | all | nothing | Marker |
| every other ability | self kinds; areas | instant: silly; lasting: added once (areas: to the room's targets with each one's element share) | Lasting |

### Before the effects

- A typed cast by a confused caster is wasted; a dead or afraid caster casts nothing; Mage Bind
  ("Your spell fails!") and, for a Kai class, Kai Bind ("Your power fails!") stop a typed cast.
- In a protected room, outside a tournament and sysop debug, an attack type or an EVILINCOMBAT
  spell is refused and costs the energy.
- Casting (not a chain) ends resting, sneaking and hiding.
- The targets are counted ([`super::is_valid_target`], [`super::is_valid_monster_target`]) and
  every player rolls to resist ([`super::resist::player_resists`]): a resisting player is told
  and not counted; a non-General spell ends the resting and sneaking of every valid player. No
  KILLDEAD, AFFECTSANIMAL, AFFECTSLIVING or Spell Immunity gate exists in this engine. No
  target at all: "Your spell has no effect in this room!", nothing spent.
- An item or chain cast of a spell with a magery the class does not have at that level does
  nothing; otherwise it always succeeds.
- An evil attack type warns as EVILINCOMBAT does.
- A typed instant attack type outside auto-combat starts combat ("*Combat Engaged*") and is cast
  by auto-combat on later rounds.
- A Kai power never fails. A typed General spell needs the once-per-round cast ("You have
  already cast a spell this round!") and succeeds when the difficulty is at least 200 or the
  roll is under `min(spellcast + diff, 98)`; attack types roll the same with no per-round limit.
- A failed roll: "You attempt to cast X, but fail.", the energy and half the mana are spent, and
  the steps after the effects still run. A success spends the energy and the mana.

## B.1 `cast_user_target`: a player on a player

The single kinds that name a player (Single, Self or Single, Any, Monster or Single: "single4")
reach the effects; any other kind is "You may not cast that spell on a user!". The strength is
rolled as for `cast_no_target`, then the target's element resistance is taken off once; a slot
with its own value replaces it (without the element cut). One success message and one lasting
add per cast.

| Ability | Targets | What happens | Variant |
|---|---|---|---|
| EVILINCOMBAT | before the attempts | outside a safe arena: a General spell must pass the player-killing level range (or the caster is in sysop debug), else "unbalanced combat round"; then the evil points are charged, and a refusal stops the cast | Marker |
| NONMAGICAL | before the attempts | the Spell Immunity gate is skipped | Marker |
| HOLDPERSON, PARALYZE | before the attempts | a target with Freedom is immune: every attempt counts as resisted (energy and half the mana spent). The effect itself is a lasting spell. | Lasting |
| POISON | before the attempts | a poison-immune target is immune; so is one hit by a lasting poison-element attack spell | Poison |
| COMPONENT | before the attempts and the energy test | as in `cast_no_target` | Component |
| REMOVESPELL, TERMSPELL | after the roll, before the other slots | the target's first active spell with the number ends (with its "Cast on ending" for REMOVESPELL) and nothing else of the spell happens | RemoveSpell |
| TEXTBLOCK | all | runs for the target; one success message | TextBlock |
| CLEARITEM | all | every copy of the item leaves the caster's room; no message | ClearItem |
| TELEPORTROOM, TELEPORTMAP | all | one half of the destination; the target is moved after the effects | Teleport |
| SHATTER | all, any duration | the target's wielded weapon breaks when the amount is above its Quality: it is unwielded and, outside a safe arena, destroyed with its messages; in a safe arena only unwielded ("would have shattered") | Shatter |
| REMOVECURSE | Self or Single and Any, instant (areas silly; Single and Monster or Single do nothing; lasting silly) | the target's worn items that are Cursed, or whose Major Curse level is below the caster's level, come off | RemoveCurse |
| FREEDOM | single4, instant (lasting silly) | every active spell of the target with Hold person or Paralyze ends | Freedom |
| DISPELMAGIC | single4, instant (lasting silly) | every active spell of the target with the ability the value names (-1: all) ends | DispelMagic |
| DAMAGENOARMOUR | single4 | instant: the amount scaled by the caster's spell damage bonus comes off the hit points; a kill pays no experience; else the knock-out rule. Lasting: the spell is added with the unscaled value. No magic resistance. | Damage{ignore_armour:true} |
| SLAY | single4, instant (lasting silly) | the hit points fall to one below death (from below 0: to death) | Slay |
| DAMAGEARMOUR | single4, any duration | the spell damage bonus added inline, then the player magic-resistance multiplier | Damage{ignore_armour:false} |
| ENSLAVE | all | silly: meaningless on a player | Enslave |
| STEALHP | single4, instant (lasting silly) | the target loses the amount and the caster gains it, capped at his maximum; no bonus, no resistance | StealHp |
| ENERGY | single4 | instant: energy changes by the amount; lasting: added once | Energy |
| SUMMON | single4, instant (lasting silly) | the monster is generated following the target, hostile | Summon |
| HUNGER, THIRST | single4 | instant: the field changes, no message; lasting: added once | Hunger, Thirst |
| HEAL | single4 | instant: hit points rise to at most the maximum; the message shows the rolled strength; lasting: added once | Heal |
| HEALMANA | single4 | instant: mana rises to at most the maximum, never below 0; lasting: added once | HealMana |
| POISON | single4 | instant: the poison level is raised to the amount if lower; lasting: the raise and the add | Poison |
| CUREPOISON | single4 | instant: the first poison spell ends, then the poison falls by the amount, never below 0; lasting: added once | CurePoison |
| GIVESPELL | single kinds and Self, lasting only (instant and areas silly) | the add is made without renewal; when it took (or an earlier slot added the spell) the spell joins the target's spellbook until the spell ends | GiveSpell |
| ALTERHP, INTELLECT .. CHARM | single kinds and Self, lasting only (instant and areas silly) | as in `cast_no_target`, on the target | AlterHp, StatAlter |
| ENDCAST, ENDCASTCHANCE | all | an instant cast chains its spell at the target unless the target died | EndCast, EndCastChance |
| the markers, KILLDEAD, DETECTMAGIC, NONMAGICAL | all | nothing | Marker |
| every other ability | single4 | instant: silly; lasting: added once | Lasting |

### How a cast at a player goes

- The resist roll: a worn item that negates the spell resists always; otherwise a spell that may
  be resisted (always, or with Anti Magic for the Anti Magic mode) is resisted when the cast
  succeeded and `genrdn(1,100)` is at most `min(mr/2, 98)`. A resisted or failed attempt costs
  the energy and half the mana; a working one the energy and the mana.
- The strength: as `cast_no_target`, then the target's element resistance is taken off once;
  the caster's spell damage bonus is applied only by DAMAGENOARMOUR (scaled) and DAMAGEARMOUR
  (inline).
- An attack type at oneself is refused ("Why would you want to attack yourself?"); any other
  spell at oneself goes to `cast_no_target`, except an Any spell, which stays here.
- Before the attempts: a confused, dead or afraid caster, Mage Bind and Kai Bind stop the cast
  as in `cast_no_target`; resting and sneaking end (the target's too unless the spell is
  General); a protected room refuses attack types and EVILINCOMBAT spells; an item cast needs
  the player-killing range for attack types and the class's magery, and pays the evil points;
  the target must be in the caster's room; the caster's level must reach the spell's ("too
  powerful for you to control").
- Spell Immunity: outside auto-combat and unless NONMAGICAL, a target whose immunity is above
  the spell's level is not affected ("Your spell has no effect on X.").
- Up to 20 attempts: a General spell takes one (needing the once-per-round cast); an instant
  attack spell typed outside auto-combat starts combat, warns the target and is cast by
  auto-combat later.
- After the effects: the teleport (unless the target died), the chain, and the target's
  secondary stats.

## B.2 `cast_item_target`: a player on an item

Only the Item and Any kinds may be aimed at an item ("You may not cast that spell on an
item!"). There is no living target, so no resistance, Spell Immunity, element cut, spell damage
bonus, protected room, level range or evil points. The strength is rolled as for
`cast_no_target`; a slot's own value replaces it.

| Ability | Targets | What happens | Variant |
|---|---|---|---|
| COMPONENT | after the success roll | as in `cast_no_target`, spent even when the roll failed | Component |
| DAMAGENOARMOUR, DEFENCE, COLDDEFENCE, DAMAGERESIST, STEALHP, SHADOWSTEALTH, SHIELD, DAMAGEARMOUR, SHADOWFORM | all | refused as silly (the cast was still paid for) | Damage, StealHp, Lasting |
| CLEARITEM | all | every copy of the item number leaves the caster's room, not the targeted item (sic) | ClearItem |
| DAMAGE, HOTDEFENCE, ENSLAVE, ENERGY, SUMMON, USERLIGHT, ROOMLIGHT, HUNGER, THIRST, HEAL, POISON, CUREPOISON, POISONIMMUNE, AVBLESS, KILLDEAD, DEFVSEVIL, BLINDINGLIGHT, AVCURSE, AVTHIRD, HEALMANA | all | nothing, with no message | (their variants) |
| DETECTMAGIC | Item and Any, instant (lasting silly) | the item's "Magical" value is shown as a band: 1 "glows faintly", 2-3 "glows softly", 4-5 "glows brightly", above 5 "almost blinded by the aura", none "You detect no magic in that item!"; the room is told | DetectMagic |
| LORE | Item and Any, instant (lasting silly) | when the item's level is at most the amount, its hidden descriptions are shown (or "Despite your best efforts, you do not recall anything special."); too weak: the same "do not recall" line; the room is told | Lore |
| NONE, REMOVESPELL, TERMSPELL | all | nothing | Marker |

One attempt only; a failed roll spends the energy and half the mana. No chain, no teleport.

## C.1 `cast_monster_target`: a player on a monster

Only the Monster, Monster or Single and Any kinds reach a monster. The strength is rolled as for
`cast_no_target`, then the monster's element resistance (attack types only) is taken off; a
slot's own value replaces it without the cut. One message and one lasting add per cast; the
lasting add ([`super::add_spell_to_monster`]) gets the caster's duration ([`super::duration`]).

| Ability | Targets | What happens | Variant |
|---|---|---|---|
| NONMAGICAL | before the effects | the Spell Immunity gate is skipped | Marker |
| KILLDEAD | before the effects | a monster that is not undead is not affected ("Your spell has no effect on X.") | KillDead |
| AFFECTSANIMAL | before the effects | only an "Animal" monster is affected | Marker |
| AFFECTSLIVING | before the effects | a "Non Living" monster is not affected | Marker |
| ENSLAVE | before the effects | the monster's charm resistance replaces its magic resistance for the resist roll and DAMAGEARMOUR; a charm resistance of 0 falls back to the magic resistance | Enslave |
| EVILINCOMBAT | before the effects | outside a safe arena, a Good or Lawful Good monster not following the caster charges the evil points (a refusal stops the cast); then, unless charmed or an angel, the monster may turn on the caster | Marker |
| COMPONENT | before the effects | as in `cast_no_target` | Component |
| GUARDED | an instant attack spell typed outside auto-combat | read from the monster: its guards (known monster the value names) in the room follow the caster, the last one takes the attack, and auto-combat starts | Guarded |
| CLEARITEM | all | as for a player | ClearItem |
| DAMAGENOARMOUR | any duration | the amount scaled by the spell damage bonus comes off the monster at once (a lasting spell is never added); a kill pays `exp * level` and ends auto-combat on it; a survivor may turn on the caster | Damage{ignore_armour:true} |
| SLAY | instant (lasting silly) | the monster falls to -2 hit points | Slay |
| DAMAGEARMOUR | any duration | the bonus added inline, then the monster multiplier ([`super::resist::monster_mr_damage`]), signed: a monster below 50 takes extra damage | Damage{ignore_armour:false} |
| ENSLAVE | any duration | when the caster's level reaches the monster's charm level: the monster follows the caster as a charmed friend; a lasting spell is also added. Too low: nothing, no message | Enslave |
| STEALHP | M/MS/Any | instant: the monster loses the amount and the caster gains it, capped at his maximum; lasting: added to the monster | StealHp |
| ENERGY | M/MS/Any | instant: energy changes; lasting: added; no message | Energy |
| SUMMON | instant (lasting silly) | the monster is generated hunting the target monster, unfriendly, filling every free hunter slot of the target (sic) | Summon |
| USERLIGHT, ROOMLIGHT, HUNGER, THIRST, BLINDINGLIGHT, HEALMANA | all | nothing on a monster | Marker |
| HEAL | M/MS/Any | instant: up to the known monster's maximum; when something was healed, a monster that is not an angel befriends the caster on a roll above its follow percentage; lasting: added | Heal |
| POISON | M/MS/Any | the immune are skipped; the poison is raised to the amount, and the monster may turn on the caster; lasting: the raise, then the add | Poison |
| CUREPOISON | M/MS/Any | instant: the poison falls by the amount, never below 0, and the monster may befriend the caster; no poison spell ends; lasting: added | CurePoison |
| ENDCAST, ENDCASTCHANCE | all | the chain at the monster unless it died | EndCast, EndCastChance |
| the markers, KILLDEAD, DETECTMAGIC, REMOVESPELL, TERMSPELL, TEXTBLOCK | all | nothing | Marker |
| every other ability | M/MS/Any | instant: silly; lasting: added once | Lasting |

### How a cast at a monster goes

- Confusion, death or fear, Mage Bind and Kai Bind stop the cast as for the other engines; the
  monster must be in the caster's room and the caster's level must reach the spell's; a
  protected room refuses it; a monster with "Required to hit" needs the matching spell
  ([`super::spell_can_hit_monster`]); an item cast of a magery spell does nothing, otherwise
  always succeeds, and attack types charge the evil points.
- Spell Immunity as for a player, from the monster's total.
- The resist roll uses `min(magres/2, 98)` with the monster's magic resistance (at least 1),
  or its charm resistance for an enslaving spell. A resisted or failed attempt costs the energy
  and half the mana.

## C.2 `monster_cast_area` and `cast_self`: a monster on the room or itself

`monster_cast_area` takes every kind but the four single ones that name a player.

- The cast roll comes first: the attack slot's cast percentage (100 from no slot). The monster
  needs the slot's energy, else nothing happens. Failure costs half the energy (at least 1);
  success all of it.
- Every player in the room rolls to resist ([`player_resists_monster_cast`], cap 97); the rest
  are marked and counted. Nobody marked: nothing is cast. The marks are not cleared after the
  cast, so an earlier cast's marks may still be there.
- The strength ([`roll_monster_amount`]) uses the monster's cast level: the attack slot's
  maximum (`atkmaxs`) from `monster_cast`, the "spells after combat" level from
  `cast_by_number`. No element cut and no spell damage bonus here.
- Full areas give each player the whole roll; every other kind divides it by the number of
  marked players.
- The monster's own lasting effects get [`monster_duration`]; players get the record's duration
  as it is. A spell already on a player is renewed only by a stronger one; a refusal before any
  success ends the cast.
- REMOVESPELL and TERMSPELL first end the named spell on every marked player
  ([`apply_monster_area_remove_pass`]) and take his mark off.
- A slot's own value reaches only the effects on the caster and the room (CLEARITEM, SUMMON,
  the caster's HEAL and CUREPOISON, its lasting effects); every per-player effect uses the
  divided base roll (sic).
- The players' element resistance comes off their share only for DAMAGENOARMOUR,
  DAMAGEARMOUR, STEALHP, ENERGY and the lasting add.

"Self" below is Self, Self or Single and Any (and Monster where listed); "area" the seven area
kinds.

| Ability | Targets | What happens | Variant | silvermere |
|---|---|---|---|---|
| CLEARITEM | all | every copy of the item leaves the monster's room (the undivided roll) | ClearItem | `apply_monster_cast_area` |
| DAMAGENOARMOUR | self and Monster; area | self: a lasting effect on the caster only. Area, instant: (sic) only Divided Area Incl. You, Full Party Area and Divided Party Area hurt anyone: each marked player loses his element share (no resistance, no bonus), with the kill and knock-out rules. Area, lasting: added to the players once. | Damage{ignore_armour:true} | `apply_monster_cast_area` |
| DAMAGEARMOUR | self and Monster; area | self: lasting only, on the caster. Area, any duration: each marked player's element share through the player multiplier, the cut cleared per player; never added as lasting | Damage{ignore_armour:false} | `apply_monster_cast_area` |
| STEALHP | area, instant | each marked player loses his element share and the monster gains it, capped at its active maximum; lasting and self kinds: nothing | StealHp | `apply_monster_cast_area` |
| ENERGY | area | instant: each marked player gains his element share; lasting: added once; self kinds: nothing | Energy | `apply_monster_cast_area` |
| SUMMON | self, instant | the known monster the roll names is generated, with nothing set on it; area kinds and lasting: nothing | Summon | `apply_monster_cast_area` |
| HEAL | self and Monster; area | self instant: the caster heals by the undivided roll up to its active maximum; self lasting: on the caster. Area instant: each marked player heals by the share up to his maximum; lasting: added once. No element cut. | Heal | `apply_monster_cast_area` |
| HEALMANA | area | instant: a positive share up to the maximum, a negative one never below 0; lasting: added once; self kinds: nothing (a monster has no mana) | HealMana | `apply_monster_cast_area` |
| POISON | self and Monster; area | self: lasting only, on the caster (no poison set). Area: each marked player without immunity is raised to the share; lasting: the raise, then the add for everyone marked. No element cut. | Poison | `apply_monster_cast_area` |
| CUREPOISON | self; area | self, any duration: the caster's poison falls by the undivided roll, never below 0 (the message names the share, sic). Area instant: each marked player (sic) without immunity loses the share, no floor and no poison spell ended; lasting: added once | CurePoison | `apply_monster_cast_area` |
| NONE, ENSLAVE, HUNGER, THIRST, KILLDEAD, DETECTMAGIC, INTELLECT .. CHARM, EVILINCOMBAT, DISPELMAGIC, AFFECTSANIMAL, FREEDOM, REMOVECURSE, the markers, REMOVESPELL, TEXTBLOCK, TERMSPELL | all | nothing when a monster casts them on the room ([`area_no_effect`]) | (their variants) | `apply_monster_cast_area` |
| every other ability | self and Monster; area | self: a lasting effect on the caster; area: added to the players once; Item and Monster or Single: nothing | Lasting | `apply_monster_cast_area` |
| POISON (`cast_self`) | lasting only | after the spell goes on the monster for the record's duration, each POISON slot adds its value to the monster's poison (no maximum, no immunity) | Poison | `apply_monster_self_spell` |
| SUMMON (`cast_self`) | instant only | the known monster the slot names is generated, with nothing set on it | Summon | `apply_monster_self_spell` |

`cast_self` has no cast roll, no resistance and no energy cost; it always shows the success
message, and a lasting spell goes on the monster for the record's duration.

Compared with the player engines: no Spell Immunity, KILLDEAD, AFFECTSANIMAL, AFFECTSLIVING,
COMPONENT, Hold or Paralyze refusal or evil points; the resist cap is 97; DAMAGENOARMOUR and
DAMAGEARMOUR get no spell damage bonus; a summon is not charmed; HEAL caps at the active record's
maximum; the self POISON adds instead of raising and ignores immunity; CUREPOISON skips immune
players, has no floor and ends no poison spell.

## D.1 `monster_cast`: a monster on a player

Only the four single kinds that name a player ("S1": Single, Self or Single, Any, Monster or
Single) are cast here; every other kind is handed to `monster_cast_area`.

- The cast roll is under the attack slot's cast percentage (100 from no slot), for every spell
  type. The player stops resting and meditating and stops leaving.
- Spell Immunity: a player whose immunity is above the spell's level resists.
- A monster without the slot's energy does nothing (its turn is used).
- A worn item that negates the spell resists; otherwise a spell that may be resisted is resisted
  on a cast that would succeed when `genrdn(1,100)` is at most `min(mr/2, 97)`
  ([`player_resists_monster_cast`]). Resisted ("You resisted X's cast of Y.") or failed: half the
  energy (at least 1). Success: all of it.
- The strength uses the attack slot's maximum as the level (no level cap; no growth from no
  slot), less the player's element resistance (attack types).
- The rounds: the record's duration plus `durinc` for every `durlvls` of the slot's maximum (from
  a slot only; [`monster_duration`]); no re-roll and no "Alter spell duration".
- A lasting spell goes on through [`super::monster_add_spell_to_character`]: renewed only by a
  stronger value; any refusal gives the energy back and ends the cast. One add per cast.
- A slot's own value replaces the strength; all ten slots are walked.
- The teleport comes after the slots; then the player's secondary stats.

| Ability | Targets | What happens | Variant | silvermere |
|---|---|---|---|---|
| TEXTBLOCK | all | runs for the player | TextBlock | `apply_monster_cast` |
| CLEARITEM | all | every copy leaves the monster's room | ClearItem | `apply_monster_cast` |
| TELEPORTROOM, TELEPORTMAP | all | half of the destination; the move after the slots | Teleport | `apply_monster_cast` |
| REMOVECURSE | Self or Single and Any, instant (Single and Monster or Single: nothing) | every worn Cursed item comes off; no Major Curse clause, no message | RemoveCurse | `apply_monster_cast` |
| FREEDOM | S1, instant | as the player engine | Freedom | `apply_monster_cast` |
| DISPELMAGIC | S1, instant | every active spell ends, whatever the value | DispelMagic | `apply_monster_cast` |
| DAMAGENOARMOUR | S1 | instant: the value comes off the hit points, no bonus; a death ends the cast; else the knock-out rule. Lasting: added once | Damage{ignore_armour:true} | `apply_monster_cast` |
| SLAY | S1, when the record has no duration | to one below death; lasting: nothing | Slay | `apply_monster_cast` |
| DAMAGEARMOUR | S1, any duration | the player multiplier with no bonus; the message shows the unsoftened value; never added as lasting | Damage{ignore_armour:false} | `apply_monster_cast` |
| STEALHP | S1, instant | the player loses the value and the monster gains it, capped at the known monster's maximum; lasting: nothing | StealHp | `apply_monster_cast` |
| ENERGY | S1 | instant: energy changes, with a message; lasting: added | Energy | `apply_monster_cast` |
| SUMMON | S1, instant | the monster is generated within the caster's own generation index, following the player, hostile | Summon | `apply_monster_cast` |
| HUNGER, THIRST | S1 | instant: the field changes, no message; lasting: added | Hunger, Thirst | `apply_monster_cast` |
| HEAL | S1 | instant: up to the maximum; lasting: added | Heal | `apply_monster_cast` |
| HEALMANA | S1 | instant: a positive value up to the maximum, a negative one never below 0; lasting: added | HealMana | `apply_monster_cast` |
| POISON | S1 | the immune are skipped; instant: raised to the value; lasting: added, then raised | Poison | `apply_monster_cast` |
| CUREPOISON | S1 | instant: the first poison spell ends and the poison falls by the value, with no floor; lasting: added | CurePoison | `apply_monster_cast` |
| ALTERHP, INTELLECT .. CHARM | S1 and Self, lasting only | the add without renewal; when it took (or an earlier slot added the spell) the stat rises; a refusal gives the energy back and ends the cast | AlterHp, StatAlter | `apply_monster_cast` |
| KILLDEAD, DETECTMAGIC, NONE, ENSLAVE, EVILINCOMBAT | all | nothing | Marker | `apply_monster_cast` |
| every other ability | S1 | lasting: added once; instant: nothing (never silly here) | Lasting | `apply_monster_cast` |

`monster_cast` does nothing of its own for SHATTER, SCATTER, GIVESPELL, ENDCAST, ENDCASTCHANCE, NONMAGICAL,
COMPONENT or the markers: with a duration they make the spell lasting like any other ability.

## D.2 `room_cast_on_user` and `room_cast_on_monster`: a room spell

### On a player (`room_cast_on_user`, [`apply::apply_effect`] with `CastPath::Room`)

- There is no cast roll and no caster. The resist roll comes before the worn items are looked at
  ([`player_resists_room_cast`], cap 97); either resists silently.
- No Spell Immunity, no KILLDEAD, AFFECTSANIMAL or AFFECTSLIVING filter, no REMOVESPELL or
  TERMSPELL pass, no COMPONENT, no Hold or Paralyze refusal.
- The strength is uniform in the record's own `[minval, maxval]` (no level), less the player's
  element resistance; a slot's own value replaces it. Only slots holding an ability apply.
- The duration is the record's own. A lasting effect is added once, at the end of its slot, with
  renewal and its result unchecked; the stat alters add without renewal and leave the stat alone
  when that fails.
- The teleport comes last.

| Ability | Targets | What happens | Variant |
|---|---|---|---|
| TEXTBLOCK, SCATTER, CLEARITEM | all | the text block runs; the room's contents move to the room the amount names; every copy of the item leaves the room | TextBlock, Scatter, ClearItem |
| TELEPORTROOM, TELEPORTMAP | all | half of the destination | Teleport |
| SHATTER | all, any duration | the weapon breaks when the amount beats its Quality; always destroyed (no safe arena) | Shatter |
| REMOVECURSE | Self or Single and Any, instant | every worn Cursed item comes off (no caster, so no Major Curse clause) | RemoveCurse |
| FREEDOM, DISPELMAGIC | S1, instant | as the player engine | Freedom, DispelMagic |
| DAMAGENOARMOUR | S1 | instant: the amount comes off, no bonus; no kill check, only the knock-out of a player alive before; lasting: added | Damage{ignore_armour:true} |
| SLAY | S1, when the record has no duration | to one below death; knock-out only | Slay |
| DAMAGEARMOUR | S1, any duration | `amount * max(150 - mr, 25) / 100` ([`super::resist::room_mr_damage`]), no Anti Magic branch; knock-out only | Damage{ignore_armour:false} |
| STEALHP | S1, instant | plain damage; nobody gains | StealHp |
| ENERGY, HUNGER, THIRST, HEAL, HEALMANA, POISON, CUREPOISON | S1 | as the player engine (Hunger and Thirst with no message) | (their variants) |
| SUMMON | S1, instant | generated following the player, hostile | Summon |
| ALTERHP, INTELLECT .. CHARM | S1 and Self, lasting | as the player engine with no caster | AlterHp, StatAlter |
| the markers, ENSLAVE, KILLDEAD, DETECTMAGIC, REMOVESPELL, NONMAGICAL, TERMSPELL | all | nothing; listed so they do not make the spell lasting | Marker |
| every other ability | all | lasting when the record has a duration | Lasting |

### On a monster (`room_cast_on_monster`, `apply_room_on_monster`)

Only the Monster, Any and Monster or Single kinds ("MON3") reach a monster.

- Gates first ([`apply::monster_gate`]): KILLDEAD needs an undead monster, AFFECTSANIMAL an
  "Animal" one, AFFECTSLIVING refuses a "Non Living" one; any of them ends the cast.
- ENSLAVE makes the charm resistance the resistance ([`room_charm_resistance`]); there is no
  fallback, so without it nothing resists.
- Spell Immunity: a monster whose immunity is above the spell's level is not affected.
- The resist roll: `min(charmres/2, 98)`; the monster's own magic resistance is never read.
- The strength: the record's own range, less the element resistance, held to `maxval`
  ([`room_monster_power`]); a slot's own value is held to `maxval` too. The slots stop once the
  monster is killed.
- `spelladded` is never cleared between monster casts (sic); only a cast on a player resets it.
  While it is set the lasting blocks and the POISON add are skipped.

| Ability | Targets | What happens | Variant |
|---|---|---|---|
| DAMAGENOARMOUR | MON3, any duration | the amount comes off; a kill ends auto-combat on it; no bonus, no turn-on-caster roll, no experience | Damage{ignore_armour:true} |
| SLAY | MON3, any duration | to -2 hit points | Slay |
| DAMAGEARMOUR | MON3, any duration | `amount * max(150 - charmres, 25) / 100`, held to `maxval` | Damage{ignore_armour:false} |
| ENERGY | MON3 | instant: energy changes; a lasting Energy is dropped | Energy |
| SUMMON, CUREPOISON | MON3 | nothing but the flag | Marker |
| HEAL | MON3 | never instant: the lasting block, whatever the duration | Heal |
| POISON | MON3 | the immune are skipped; raised to the amount; a lasting one also takes the amount off the hit points at once (no kill check); then the spell is added (first free slot, the record's duration), even without a duration | Poison |
| the markers, USERLIGHT, ROOMLIGHT, HUNGER, THIRST, BLINDINGLIGHT, TEXTBLOCK, HEALMANA, REMOVESPELL, TERMSPELL, KILLDEAD, DETECTMAGIC | all | nothing ([`room_monster_no_effect`]) | Marker |
| every other ability | MON3 | the lasting block: the slot holding the spell is renewed with the record's duration and the amount, then every free slot is written too (sic); afterwards the slot loop goes on at slot 6, whatever slot it was on (sic; `apply_room_on_monster_spell`) | Lasting |

## E. Per-round and end-of-spell effects

The background cycle runs these for every spell in a slot, slot by slot in record order. Each
slot uses its own value, or the strength the spell was cast with.
[`apply::terminate_user_poison_spell`] ends a player's first poison-element spell with its
ending (chain allowed).

### Each round on a player (`apply_round`, `apply_round_slot`)

| Ability | What happens | Variant |
|---|---|---|
| FLEE | with the value in 100 the player runs out through a random usable exit | Lasting(FLEE) |
| DAMAGENOARMOUR | the hit points fall by the value; the knock-out rule | Damage{ignore_armour:true} |
| STEALHP | the same; nobody gains | StealHp |
| ENERGY | rises to at most the maximum | Energy |
| HUNGER, THIRST | change by the value | Hunger, Thirst |
| HEAL | rises to at most the maximum | Heal |
| HEALMANA | kept between 0 and the maximum | HealMana |
| CUREPOISON | the poison falls by the value, never below 0; a negative value is ignored | CurePoison |
| everything else | nothing each round | |

DAMAGENOARMOUR, STEALHP, HEAL and HEALMANA send the prompt.

### When a player's spell ends (`apply_termination`)

The ending message comes first (the engine's); after the slots the chain and the secondary stats.

| Ability | What happens | Variant |
|---|---|---|
| GIVESPELL | the given spell leaves the spellbook | GiveSpell |
| POISON | the poison falls by the value, never below 0 | Poison |
| ALTERHP | the maximum and current hit points fall by the value | AlterHp |
| INTELLECT .. CHARM | the stat falls by the value | StatAlter |
| ENDCAST, ENDCASTCHANCE | when the ending may chain, the spell is cast with the player as caster (a chain cast) with the chance (default 100) | EndCast, EndCastChance |
| everything else | nothing | |

### Each round on a monster (`apply_monster_round`)

| Ability | What happens | Variant |
|---|---|---|
| FLEE | with the value in 100 the monster runs out through a random usable exit | Lasting(FLEE) |
| DAMAGENOARMOUR | the hit points fall by the value | Damage{ignore_armour:true} |
| ENERGY | rises to at most the maximum (compared unsigned) | Energy |
| HEAL | rises by the value, with no cap | Heal |
| CUREPOISON | as for a player | CurePoison |
| STEALHP, POISON, HEALMANA, everything else | nothing each round (a monster is not drained and has no mana) | |

### When a monster's spell ends (`apply_monster_termination`)

| Ability | What happens | Variant |
|---|---|---|
| ENSLAVE | the monster goes free: it follows nobody, is no friend and is no longer charmed | Enslave |
| POISON | the poison falls by the value, never below 0 | Poison |
| everything else | nothing | |
