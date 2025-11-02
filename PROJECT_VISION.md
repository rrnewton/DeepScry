
A performant Magic The Gathering engine for AI research
================================================================================

Forge and Xmage are two open source projects that have done the hard work of implementing the rules engine for the Magic The Gathering (MTG) game, plus curating a full card library (>30,000 cards) to capture the behavior of all existing MTG cards.
   
   https://github.com/Card-Forge/forge
   https://github.com/magefree/mage

In this git repo, you'll find a submodule for forge. The goal of this project is to port the game engine from that Java-based Forge project, to a new implementation in Rust, while making certain improvements.

The main goal of the improvements is to make this new implementation suitable for AI research which requires being able to aggressively search future subtrees of the game decision tree. As detailed below this will require some cross-cutting upgrades to the way game entities are implemented.

Porting strategy
----------------------------------------

We want to build a game engine by porting the Java code, and reuse the card library and its text-based format for representing cards, for example:

```
$ cat ./forge-java/forge-gui/res/cardsfolder/l/lightning_bolt.txt
Name:Lightning Bolt
ManaCost:R
Types:Instant
A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3 | SpellDescription$ CARDNAME deals 3 damage to any target.
Oracle:Lightning Bolt deals 3 damage to any target.
```

For brevity this `cardsfolder` is symlinked into the top of this repo as well. A
major component of this project will be building and testing the loader/parser for these card files.  Another major input format to read are the `.dck` deck files stored throughout the repository.

We should port the Forge project file-by-file, and feature-by-feature, but not necessarily line-by-line. Our port will be less direct for a couple reasons. First, we are dropping functionality and adding new functionality. Second, we want the result to be grown incrementally and testable/benchmarkable at each step.

### Dropping functionality: Leave out the GUI.

The goal of this project is the core game-engine and eventually AI/AI or AI/human gameplay. We are NOT concerned with implement any graphical user interface either for desktop, web, or mobile. Rename everything "GUI" to "UI": for example, the forge-gui package would probably become a forge-ui crate.

In the `./forge-java` submodule, iou can see a package `forge-headless`, which is a work-in-progress TUI for playing MTG on the command line. This is a great way to test the engine and any AI under development. We will adopt the same text interface, though we will extend it as we surpass the functionality of the WIP forge-headless prototype. Note that the key design of the TUI is to reduce the game of MTG to an explicit decision tree of numerical `0-N` choices at each branch point. Therefore a complete game can be written down as a series of small integers (for both players) to navigate the decision tree.


### Look for opportunities for Rust features like traits and polymorphism

Rust doesn't directly include the object oriented (OOP) features of class-based inheritance that Java does. Nevertheless, it is usually possible to create Trait/Impl hierarchies matching the Class/Instance types that we start with.

From a typing perspective, we want to make the types relatively "tight" fitting where we don't expose operations on objects which are invalid for that type of object. There may be places where the Java implementation has exposed operations too high in its inheritance hierarchy and they don't make sense for all subclasses. For example, `GameEntity` exposes quite a few methods which may not make sense for every potential game entity.

We want to keep the types *relatively* simple on the Rust side. Often, Rust libraries have extremely complex combinations of traits, parametric polymorphism, and callback interfaces.

The types we are starting with on the Java side are straightforward. We will mostly translate them to straightforward Rust counterparts. But we will look for opportunities where sprinkling in a little bit of fancier types has a good power-to-weight ratio---that could mean, for example, making some of our types take parameters, or using train-bound dynamic dispatch.

### Keep the simple concept of GameEntity IDs

The Java-based game uses a simple integer ID for every GameEntity and we will keep this design. These are useful even in the TUI for disambiguation; for example, if "Birds of Paradise (81)" goes to the graveyard, and two were on the battlefield, you know which one it was.

Keep these IDs simple and contiguous. This is better for human readbility and also allows them to be stored in dense structures like Vec's if desired (i.e. with index=cardID as the "key").

One great thing about the MTG game is that the entities over the course of the game are MOSTLY static and not frequently dynamically allocated or freed. As the game evolves:

 - We start with all the cards in our deck, and no further ones appear (aside from tokens).
 - Cards move from the library, to the hand, to the stack, and to the battlefield/graveyard/exile.

GameEntity ownership can reside in a single entity hashmap or vector per game.  We should abstract this EntityStore datatype in case we want to change it for the future.

Fast changing of the game state should amount to mutating the fields, and the pointer graph (expressed by IDs not by physical machine pointers), such as when pushing a card effect onto the stack.

### Counters and SpellAbilities

We see more dynamism with tokens, counters, and SpellAbilities, which appear and disappear. When porting these concepts over we will try to AVOID the pattern of heap-allocating and deallocating large chains of nested collections rapidly.

One tactic is to use `smallvec` and `small-map` crates anywhere we have expected-small collections. For example, for counters we can follow the same basic arrangement as the Java code, where we have a large enum (CounterEnumType) plus CounterKeywordType, and every GameEntity has a map of counters, but that map can be stored as a `small-map::SmallMap`.

We will keep on the lookout for sources of dynamic allocation:
 - CardDamageMap, CardZoneTable, GameEntityCounterTable, CardCollection
 - Combat, ManaCostBeingPaid, CostPayment, CostPaymentStack

Some heap allocation is probably unavoidable, but where possible we should unbox objects and use stack-allocation. For instance, a basic advantage of Rust is that a vector of enum values (even including data fields / full tagged unions), remain unboxed, rather than being a vector of pointers to objects like in Java. Another example of a performance anti-pattern to avoid is return a freshly allocated `Vec<T>` when it is better to simply retrieve an iterator over the existing memory (whose lifetime matches the original reference containing the collection in question).

## Incrementally growing the engine and gameplay

We have a very large porting task ahead of us. It's essential that we make incremental progress and create testable mini-prototypes from the beginning of the project.  We will growing our port incrementally while testing. Below in the testing section we discuss some ideas for a basic MVP starting point.

Improvements: Rewind ability and Performance
--------------------------------------------

Above, we already covered opportunities for unboxing and more efficient data representations. This will be one source of performance improvements.

But the major change will be to enable searching the game tree by moving forward AND backward it time. This is new functionality, but could also be viewed as a massive performance improvement, because any attempt to do it with the Java codebase now would probably require an expensive deep copy of the heap-based GameState in order to rewind.

### Rewind through Undo Logs.

In order to create an efficient data structure for the UndoLog we need to reify the concept of a GameAction into a giant Enum that we can record in an unboxed vector of actions. This is similar to the journal in a file system or transaction-log in a database.

It will take some serious iteration to identify the right granularity of action to record. For example, if we play a lightning bolt that deals three damage to a creature X, and it goes to the graveyard, what is the most efficient way to represent that and to rewind it?

We want to mark where the choice points are that correspond to player "moves" because the minimum amount to rewind is one "move", backing up to a prior choice point and making a different choice.

We should have a compile-time flag to turn OFF undo logging. This will let us benchmark how long it takes to replay a recorded game through the engine.  (Recall that a recorded game can be as simple as a vector of integers for the choice points -- even stored with binary serialization (see below).)

The opponent commits a move we can flush the undo buffer, as we use it only for looking into future game trees, and don't need to rewind all the way to the beginning of the game.

Overall, the UndoLog gives us a way to get back to a GameState by applying actions in reverse. The main GameState is mutable, but the undo-log is (mostly) append-only. An alternative design would be to make our core game state immutable, and use a purely functional programming approach to explore future game trees. However, even if this could hypothetically perform well, Rust, being malloc-based, is not suited to extremely high allocation rates.

### Architect for potential parallel games

Don't assume any global state for games. We should be able to create many simultaneous Games within the same process. As we consider parallel search strategies this ability may be important.

### Fast, binary full game snapshots

Related to the topic of parallelization. In addition to accumulating an UndoLog while making forward moves and mutating the game state, we should also be able to make efficient deep copies (Clone) of the complete game state.  This will be useful for debugging or parallelization. But it may also be useful if we find it difficult to use the UndoLog strategy for everything. Maybe certain hard-to-support game actions will require making a full GameState backup in order to rewind, rather than an efficient GameAction log.

### Serialization considerations

Aside from the special human-readable file formats we want to read and write (for cards, decks, and game states), we want all the main game types to be serialiable via `serde_json` for human debuggability and readability. Ideally we want to support derive-able serialization where possible. We also want the core types to be efficiently support binary storage as well. Let's initially attempt to use the popular `rkyv` library for efficient on-disk binary storage.  We want to never serialize/deserialize textual representations on any of our high-performance hotpaths (for tree-search and AI). Our tests can help make sure that dumping a full game state in both text and binary deserialize back to equivalent values.


Testing strategy
----------------------------------------

We want a combination of unit testing with something more like integration or e2e testing that will often test full games or combinations of them.

### Create a playable tiny MVP

Because MTG is such a large and complex game, how do we create a minimal viable product (game) that we can play? The basic idea is to start with very simple decks that exercise only a subset of th mechanics. For instance, consider:

- playing two identical decks against each other, each with 20 Mountains and 40 Lightning Bolts,
- implementing player life (starting at 20) and spell casting during the first main phase only,
- implementing random initial hand and card draw.

In this scenario, both players will simply play mountains and cast lighting bolts to damage each other. The game is a trivial subset of the full MTG game. But we should start with this minigame (or something similar) and fully build out (but partially populate) all the interoperating components that we want working together---core types, player choices, TUI, undo log, etc.

From that baseline we can then incrementally add in more of the games complexity, one piece at a time.

- creature playing
- creature targeting (e.g. with Lighting Bolt)
- creature attacking
- creature blocking
- reacting to spells on the stack (e.g. Counterspell)
- counters
- discarding
- many many other mechanics

We can use any of the `./cardcollection` or over 6000 deck (`.dck`) files to drive testing, and you can also make new test decks such as the one that consists of all lightning bolts (or all counterspells, or all simple creatures), these test decks need not fully legal MTG decks.

### Playable for humans and LLM agents like Claude

Our ability to quickly debug and deeply test functionality should depend on how easy it is to reproduce game states, including to reproduce bugs. The raw serialization formats we want to use for debugging or efficiency are not ones we want to commit to the repository for test cases. See below for the kind of high-level game state description we want to commit to test files.

Let's suppose that Claude wants to test that a counterspell is playable in a certain situation. The test case for this can load a game file at a certain phase, step the game forward and have P2 play a lightning bolt, then make sure P1 has the option of responding with a counterspell.

And when something doesn't go as expected, we want Claude to be able to jump to these intermediate states in gameplay easily. Claude cannot actually play the game through the TUI, but excels at writing scripts on the fly. So Claude could script simple gameplay by writing scripts that drive the TUI
(basically `echo 0 1 3 2 | run_tui_from_state gamestate.mtggame`), or by the TUI having a flag to read in choice-logs, or by writing Rust code in a cargo test that loads a game state and takes actions with the API.

### GameState text files

We can base our gamestate text files on a cut down version of the `.pzl` files already supported by the Java codebase. For example, these allows a notation of `activeplayer \in {p0, p1, Human, AI}`, case insensitive, but we can restrict it to `p0/p1`. Examples:

```
$ cat ./forge-java/forge-gui/res/puzzle/PC_102715.pzl
[metadata]
Name:Perplexing Chimera (GatheringMagic.com) 102715 - This Is My Broomstick
URL:http://www.gatheringmagic.com/seanuy-102715-this-is-my-broomstick/
Goal:Win
Turns:2
Difficulty:Hard
Description: Win on your next turn.
[state]
ActivePlayer=AI
ActivePhase=main2
HumanLife=14
AILife=5
humanhand=Festering Newt; Undying Evil; Wretched Banquet; Crown of Suspicion
humangraveyard=Skeletal Scrying
humanlibrary=Dead Drop
humanbattlefield=Swamp|Set:SHM; Swamp|Set:SHM; Swamp|Set:SHM; Swamp|Set:SHM; Swamp|Set:SHM; Swamp|Set:SHM|Tapped; Swamp|Set:SHM|Tapped; Polluted Mire|Tapped; Haunted Fengraf; Cackling Witch; Plague Witch; Bogbrew Witch; Bubbling Cauldron
aibattlefield=Forest|Set:AVR; Forest|Set:AVR; Forest|Set:AVR|Tapped; Forest|Set:AVR|Tapped; Forest|Set:AVR|Tapped; Plains|Set:AVR; Plains|Set:AVR; Plains|Set:AVR|Tapped; Plains|Set:AVR|Tapped; Folk of An-Havva; Loyal Sentry; Norwood Archers; Foot Soldiers; Village Survivors
aihand=
aigraveyard=
ailibrary=
aiexile=

$ cat ./forge-java/forge-gui/res/puzzle/PS_OTJ3.pzl
[metadata]
Name:Possibility Storm - Outlaws of Thunder Junction #03
URL:https://i0.wp.com/www.possibilitystorm.com/wp-content/uploads/2024/05/latest-scaled.jpg?ssl=1
Goal:Win
Turns:1
Difficulty:Rare
Description:Win this turn. Your opponent can cast their Collective Nightmare at any point. Ensure your solution takes that into account, as well as all possible blocking decisions!
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Epic Confrontation;Gold Rush
p0battlefield=Stoic Sphinx;Ornery Tumblewagg|Counters:P1P1=2;Ramosian Greatsword;Suspicious Bookcase;Bristling Backwoods;Bristling Backwoods;Botanical Sanctum;Botanical Sanctum
p1life=10
p1hand=Collective Nightmare
p1battlefield=Harvester of Misery;Swamp;Swamp;Swamp
```

We don't need the concept of goal or turn budget or difficulty. Though we may in the future implement these full Puzzles to test our AIs.

### Cargo test and Github workflows

As we code, we want to commit frequently every time we have completed a task, e.g. implemented a feature, and the code is compiling and passing tests. We are using Beads (see `bd quickstart` command) to track issues.

We want to use idiomatic `cargo test` functionality for testing our project. The tests should in general be parallelizable.

We want a github workflows configuration that runs the tests in the CI/CD system.

Benchmarking strategy
----------------------------------------

In addition to correctness testing, performance testing is important for a search-focused project like this. We will port over the basic notions of board state evaluation from forge-java, and key performance issues in search will include:

- Speed of moves: basic actions against the game state.
- Speed Board state evaluator.
- Tree search: number of states evaluated per second.

The basic metric of boardstates-per-second explored per second will provide a good indication of how we're doing.

In addition to `cargo test`, we expect `cargo bench` will run a sensible set of benchmarks and record the resulting metrics in a human-consumable way.

We want to use Criterion.rs to benchmark individual key functions in our repository. This is analogous to unit testing. But we also want e2e testing that, for example, does complete tree-search to a fixed depth look-ahead K and times our boardstates-per-second as we play forward a game in this mode, or extrapolate from a single game state in this mode.

