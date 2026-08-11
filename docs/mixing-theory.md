# Mixing Theory, for a Classical Pianist

You already know most of this. The vocabulary is different and the notation is
missing, but the underlying material — phrase structure, modulation, voice
leading, large-scale form, tension and release — is the same material you spent
years on. This doc translates.

The one genuinely new skill is at the end: composing with *spectrum* instead of
*pitch*. That's the part your training doesn't give you for free.

---

## 1. The thing that makes it click: phrase structure

Dance music is in 4/4. Not "usually" — essentially always, and if it isn't, it
isn't mixable in the beat-matched sense anyway (which is why this app detects
fluid tempo and gives classical and rubato tracks key badges instead of fake
beat grids).

Above the bar, the structure is rigid in a way classical music never is:

```
4 beats      = 1 bar
8 bars       = 1 phrase      ← the fundamental unit
16 or 32 bars = 1 section     ← intro, breakdown, drop, outro
```

This is the classical **period** — antecedent + consequent, 4 + 4 — stripped to
a skeleton and then made mandatory. Where Haydn writes a 4+4 period and then
immediately starts deforming it (a 5-bar consequent, an elided cadence, a
one-bar extension to set up the retransition), a house producer writes 8, then
8, then 8, then 8, forever, and the deformation budget is zero.

That rigidity is not a poverty. It's the *enabling condition*. It's the only
reason two records made by strangers a decade apart can be overlaid at all.
Rubato makes phrase alignment impossible; a grid makes it free.

**The practical consequence, which is the single most important rule in
mixing:** start the incoming track on a phrase boundary. Ideally a 16- or
32-bar boundary. Do that and the transition sounds inevitable. Come in three
bars early and it sounds exactly like a wind player entering three bars early —
not subtly wrong, *obviously* wrong, to anyone, whether or not they can say
why.

### The drill

Before you touch a crossfader again: play any dance track and count out loud.

```
1-2-3-4  2-2-3-4  3-2-3-4  4-2-3-4  5-2-3-4  6-2-3-4  7-2-3-4  8-2-3-4
└──────────────────────── one 8-bar phrase ─────────────────────────┘
```

Then start over. Four of those is a 32-bar section, and you'll hear the
arrangement confirm it — a new element enters, or everything drops out, right
on the boundary. Do this until you stop counting and just *feel* the 32.

You have perfect training for this and it will take you an afternoon, not a
month. Counting long hypermeter is what you already do when you play a Chopin
nocturne and know where the phrase is going. The difference is that here the
hypermeter is exact, so you can be exact back.

---

## 2. Harmonic mixing is the circle of fifths wearing a hat

DJs use the **Camelot wheel**. It is the circle of fifths with the key names
replaced by numbers so that people who don't read music can use it.

```
Camelot   Key           Camelot   Key
  1A      A♭ minor        1B      B major
  2A      E♭ minor        2B      F♯ major
  ...                     ...
  8A      A minor         8B      C major
  9A      E minor         9B      G major
 10A      B minor        10B      D major
```

- The **number** is position on the circle of fifths.
- **A** = minor, **B** = major.
- Same number, different letter = **relative** major/minor (8A = A minor,
  8B = C major — same signature).

So the famous DJ rules decode to things you learned at fourteen:

| DJ rule | What it actually is |
|---|---|
| ±1 on the wheel | Move by a fifth |
| Same number, swap letter | Relative major ↔ minor |
| +2 on the wheel | Modulate up a whole step (two fifths) |
| +7 semitones | Up a fifth — same as +1 |
| "Energy boost" +1 semitone | The last-chorus key change. Cheap, effective, obvious |

**You can throw the wheel away.** Mix Table shows both the Camelot code and the
real key name; read the key name. "F minor into C minor" is a fifth, and you
know what a fifth sounds like. You don't need to be told 4A → 5A.

### Where it stops being classical harmony

Two things to unlearn.

**First: there is no functional harmony across a transition.** When you blend
A minor into E minor, you are not modulating — there's no pivot chord, no
dominant preparation, no cadence. Both tracks are looping static harmony and
you're simply overlaying them. It works when the two loops share enough pitch
content to not clash, and that's the whole criterion. It's closer to bitonality
than to modulation.

**Second: the key detection is approximate and dance music is often modal or
harmonically static.** A track that's one bass riff on A with no third is
"A minor" to the analyzer and equally happy under a lot of things. Trust your
ear over the badge. The badge is a filter to narrow 5,700 tracks to 40, not a
verdict.

**And the honest caveat:** harmonic mixing matters *less* than phrase alignment
and *much* less than the bass swap. A key clash over a well-timed transition is
a bad two seconds. A phrase misalignment is a bad thirty seconds. Get the
counting right first.

---

## 3. EQ is voice leading (the important one)

This is the deepest correspondence and the one that will make you good quickly.

Two tracks playing simultaneously is **counterpoint between two dense
textures**. Every rule you know about part-writing exists because of one
principle: *independent lines need independent space*. Parallel fifths are
banned because parallel motion in the same register collapses two voices into
one timbre. Low thirds are muddy because the low register can't resolve close
intervals.

Now: a kick drum and a bassline occupy 40–120 Hz. A second kick and bassline
occupy the *same* 40–120 Hz. There is no space. You don't get counterpoint, you
get mud — and worse, you get amplitude summing that eats all your headroom and
makes the mix quieter and flabbier at once.

So the primary technique in all of DJing:

> **Only one track holds the bass at a time. You hand it off on a phrase
> boundary.**

This is the **bass swap**, and it's just voicing. One instrument has the bass
line; when another takes it, the first gets out of the way. You'd never write
two cellos doubling different bass lines a step apart. Don't play two.

### What each band is, in orchestral terms

Mix Table has four bands, and the crossover points were chosen for exactly this
reason:

| Band | Range | Orchestral analogue | What lives here |
|---|---|---|---|
| Low | < 200 Hz | Cello / bass / timpani | Kick, bass line. **The contested one.** |
| Low-mid | 200–800 Hz | Viola / tenor | Body of everything. Where mixes turn to mud. |
| High-mid | 800–4k | Violin / voice | Vocals, leads, snare crack, presence |
| High | > 4k | Piccolo / triangle / air | Hats, cymbals, air, sibilance |

The 200–800 band is the one nobody thinks about and the one that ruins mixes.
It's the tenor register problem: it's where every instrument has *some* energy,
so two tracks pile up there even when neither seems to be "doing" anything in
that range. When a blend sounds congested but you can't say why, it's 200–800.

### Voice-leading rules, restated as EQ moves

- **Don't double the bass.** Kill the low band on the incoming track, swap on
  the boundary. Non-negotiable.
- **Don't double the melody.** Two vocals at once is two sopranos singing
  different words. Duck the high-mids on one, or pick tracks where one is
  instrumental. (This is why instrumentals, dubs, and acapellas exist.)
- **Thin the inner voices.** Pull 200–800 on whichever track is the "bed"
  rather than the "feature."
- **Highs are cheap.** Hats and air from two tracks coexist fine — the ear
  tolerates density up top. That's why you can run both tracks' high bands open
  and it still sounds clean.

### The goal: the phantom third track

A great blend doesn't sound like two records. It sounds like one arrangement
that neither record contains — the drums of one under the pad and vocal of the
other. That's orchestration, and it's the actual art. Key compatibility and
beatmatching are just the conditions that let it be attempted.

---

## 4. Form: tension, release, and the set as one piece

### The transition, as a retransition

A 16-bar filtered build — high-pass sweeping up, drums thinning, the loop
tightening — is a **dominant prolongation**. It's a retransition. Harmonically
static, dynamically accumulating, pointed at a downbeat that everyone in the
room can feel coming and can't hear yet.

The **drop** is the tonic arrival on that downbeat.

That's it. That's the whole rhetorical device, and it's the same device as the
end of a development section. The reason a drop lands is the same reason a
recapitulation lands: you were promised, you waited, the promise was kept on a
strong beat. And the reason a badly-timed drop dies is the same reason a
premature recapitulation dies.

The filter knob in Mix Table is that gesture in one hand.

### The set as a single form

A DJ set is not a suite of pieces. It's **one piece with an arc**, and
programming it is the same problem as programming a recital: nobody wants four
consecutive climaxes, and nobody wants an hour of introspection either.

Common shapes:

```
Ramp        ▁▂▃▄▅▆▇█        steady build, one arrival. Warm-up sets.
Wave        ▁▃▅▃▅▇▅█        build, back off, build higher. The standard.
                             The pull-back is what makes the next peak read.
Plateau     ▆▇█▇█▇█▇        peak-time. Sustained, small variation.
Descent     █▇▅▄▃▂▁         after-hours, or the end of anything.
```

The **wave** is the important one and it's the least intuitive. A continuous
build flattens out — the ear normalizes, and by track eight "loud and fast" is
just the baseline and has nowhere to go. You have to *release* to build again.
This is dynamics in a Beethoven development: the subito piano before the final
crescendo is what makes the crescendo enormous.

Your library is unusually good for this. Most DJs can't drop the energy without
losing the room, because everything they own is dance music. You have Tosca,
Nina Simone, the Costes material, and actual classical — you can execute a real
descent and come back. Use it.

---

## 5. What's actually new: composing with spectrum

Here's the part your training doesn't cover.

In classical music, the primary compositional parameters are **pitch** and
**rhythm**; timbre is orchestration, applied to material that already exists.
In electronic dance music the hierarchy inverts. Harmony is often one static
loop for six minutes. What actually develops — what carries the form — is
**timbre, frequency content, and rhythmic layer density**.

A track "goes somewhere" by opening a filter, adding a percussion layer,
removing the bass, widening the stereo field. Those are the developmental
gestures. The notes don't change.

For a pianist this is genuinely disorienting, because the parameter you have
the most refined training in (pitch relationships) is doing the least work, and
the parameter you've mostly treated as decoration (spectrum) is doing the most.

Two things follow:

1. **Learn to hear frequency ranges as identifiable objects.** Not "it sounds
   dull" but "there's nothing above 8k." Not "muddy" but "too much at 300."
   This is trainable and it's the core DJ ear skill. Sweep a filter on a
   familiar track and pay attention to *what leaves*.
2. **Energy is a compositional parameter, not a volume setting.** It's density
   and spectral weight. A sparse loud track can be lower-energy than a dense
   quiet one. Mix Table's energy score is built to reflect that (weighted
   toward mastered loudness and beat strength rather than raw spectral flux —
   which is why it doesn't rate the Mozart Requiem as energetic as Sandstorm).

Also unlearn: **rubato**. Your instinct to breathe at phrase ends, to stretch
an arrival — that instinct is correct in every other music you've played and it
is fatal here. The grid is the grid. Expressiveness lives in *what you layer
and when you cut*, never in tempo.

---

## 6. A practice program

In order. Don't skip ahead — 1 and 2 are the whole game and everyone skips
them.

**1. Count phrases, no mixing.** One session. Any dance track. Count to 8 bars,
then start over, until you feel the 32 without counting. Verify against the
arrangement — the changes will land on your boundaries.

**2. Bass swap only.** Two tracks you know, similar tempo. No crossfader — set
both faders open, use only the low-band EQ. Kill the low on B, bring B in on a
phrase boundary, and hand the bass over on the *next* boundary: B's low up as
A's low comes down, over about a bar. Nothing else. This one move is most of
what a good mix is.

**3. The same two tracks, ten times.** Not ten different pairs. Ten repetitions
of one pair, until the transition is clean and you know exactly which 32-bar
boundary you want. Then vary it: swap early, swap late, swap over four bars
instead of one. You practiced this way for years already.

**4. Record and listen back.** Mix Table records the mix bus to
`~/Music/Mix Table Recordings`. What's inaudible with your hands on the
controls is obvious on playback — especially bass pileup and phrase drift. This
is the fastest feedback loop available and almost nobody uses it.

**5. Add the filter sweep.** Only once 1–4 are automatic. A 16-bar high-pass
build on the outgoing track into a bass swap on the boundary is a professional
transition, and it's three gestures.

**6. Program a full arc.** Ninety minutes, wave shape, at least one real
descent into the Costes/jazz material and back out. This is where the library
becomes an instrument.

---

## 7. Quick reference

**Before a transition, in order of importance:**

1. Are you on a phrase boundary? (16 or 32 bars. Count it.)
2. Is only one track holding the bass?
3. Are the tempos matched and *phase*-aligned, not just BPM-equal?
4. Are the keys compatible? (Fifth, relative, or same.)
5. Are two vocals fighting?
6. Where's this going in the arc — building, holding, or releasing?

**Failure modes, ranked by how bad they sound:**

| Failure | Severity |
|---|---|
| Phase drift — beats separating | Fatal, immediately |
| Coming in off the phrase | Obvious to everyone |
| Two basses at once | Mud, headroom loss, flabby |
| Two vocals at once | Confusing, sounds like a mistake |
| Key clash | Uncomfortable but survivable if brief |
| No arc across the set | Nobody notices consciously; everyone leaves |

---

## Where the app fits

- **Camelot + key name** — the circle of fifths, already computed (§2)
- **Beat grid** — the phrase counting substrate (§1)
- **4-band EQ** — the voice-leading tool; 200–800 exists for §3
- **Filter FX knob** — the retransition gesture in one hand (§4)
- **Energy score** — the arc parameter (§4, §5)
- **Fluid-tempo detection** — knows which of your tracks aren't griddable (§1)
- **Mix recording** — the feedback loop (§6.4)

Still missing, and both matter for this:

- **Phrase markers on the waveform** — 8/16/32-bar boundaries drawn as
  gridlines. This would make §1 visual instead of counted, and it's the single
  highest-value addition for learning.
- **Headphone cue (PFL)** — preview the incoming track's phrase position before
  the room hears it. Currently you're mixing blind.
