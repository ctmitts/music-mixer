/**
 * One texture, four lenses.
 *
 * The spectral history lives in a single RGBA8 ring-buffer texture:
 *   width  = N_BINS   (log frequency, A0 upward, 3 texels per semitone)
 *   height = HISTORY  (time, written at uCursor and wrapping)
 *   R = magnitude   G = pitch class   B = tonal coherence   A = transient
 *
 * Every mode is a different (screen → bin, age) mapping over that same data,
 * which is why crossfading between them costs one `mix()` rather than a second
 * render pipeline.
 */

export const VERT = `#version 300 es
precision highp float;
in vec2 aPos;
out vec2 vUv;
void main() {
  vUv = aPos * 0.5 + 0.5;
  gl_Position = vec4(aPos, 0.0, 1.0);
}
`;

export const FRAG = `#version 300 es
precision highp float;

in vec2 vUv;
out vec4 fragColor;

uniform sampler2D uHistory;
uniform vec2  uResolution;
uniform int   uBins;
uniform int   uBinsPerOctave;
uniform int   uHistoryLen;
uniform int   uCursor;       // row that the newest frame was written to
uniform float uWindowFrac;   // fraction of history on screen (the time slider)
uniform float uAgeOffset;    // frames of delay, to match output latency
uniform float uTime;
uniform int   uModeA;
uniform int   uModeB;
uniform float uBlend;        // 0 = pure A, 1 = pure B
uniform float uLevel;        // global RMS, drives bloom
uniform float uFlux;         // onset strength, drives flash
uniform float uHueRotate;    // slow psychedelic drift
uniform float uSymmetry;     // kaleidoscope fold count for the mandala
uniform float uGain;
uniform float uSaturation;

const float TAU = 6.28318530718;

// ---------------------------------------------------------------------------
// History sampling
// ---------------------------------------------------------------------------

// Manual bilinear. Each of the two time rows is wrapped independently, so the
// ring-buffer seam never blends the newest frame into the oldest — with plain
// LINEAR filtering that seam shows up as a hard spoke across the mandala.
vec4 sampleHistory(float binF, float ageF) {
  float fb = clamp(binF, 0.0, float(uBins - 1));
  int b0 = int(floor(fb));
  int b1 = min(b0 + 1, uBins - 1);
  float bt = fb - float(b0);

  // The tap is pre-device, so what we are drawing has not been heard yet.
  // Holding the read cursor back by the output latency re-aligns picture with
  // sound — negligible over HDMI, essential over AirPlay's ~2 s buffer.
  float fa = clamp(ageF + uAgeOffset, 0.0, float(uHistoryLen - 1));
  int a0 = int(floor(fa));
  int a1 = min(a0 + 1, uHistoryLen - 1);
  float at = fa - float(a0);

  int r0 = (uCursor - a0 + uHistoryLen * 2) % uHistoryLen;
  int r1 = (uCursor - a1 + uHistoryLen * 2) % uHistoryLen;

  vec4 c00 = texelFetch(uHistory, ivec2(b0, r0), 0);
  vec4 c10 = texelFetch(uHistory, ivec2(b1, r0), 0);
  vec4 c01 = texelFetch(uHistory, ivec2(b0, r1), 0);
  vec4 c11 = texelFetch(uHistory, ivec2(b1, r1), 0);

  return mix(mix(c00, c10, bt), mix(c01, c11, bt), at);
}

// ---------------------------------------------------------------------------
// Colour: Oklab, so equal numeric steps look like equal perceptual steps and
// the hue wheel does not lurch through a muddy yellow-green.
// ---------------------------------------------------------------------------

vec3 oklabToLinearSrgb(vec3 c) {
  float l_ = c.x + 0.3963377774 * c.y + 0.2158037573 * c.z;
  float m_ = c.x - 0.1055613458 * c.y - 0.0638541728 * c.z;
  float s_ = c.x - 0.0894841775 * c.y - 1.2914855480 * c.z;
  float l = l_ * l_ * l_;
  float m = m_ * m_ * m_;
  float s = s_ * s_ * s_;
  return vec3(
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
   -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
   -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s);
}

vec3 linearToSrgb(vec3 c) {
  c = max(c, vec3(0.0));
  return mix(12.92 * c, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

/**
 * The whole mapping in one place.
 *   magnitude  → lightness
 *   pitch class → hue     (so a note keeps its colour in every octave, and
 *                          harmonically related notes land near each other)
 *   coherence  → chroma   (pure tones vivid, cymbals and breath desaturate)
 *   transient  → additive white bloom
 */
vec3 spectralColor(vec4 s) {
  float mag = pow(s.r * uGain, 1.3);
  float hue = fract(s.g + uHueRotate) * TAU;
  float coh = s.b;
  float tr  = s.a;

  float L = mag * 0.95;
  // Chroma has to fall off as lightness → 1, or bright peaks clip to a flat
  // neon slab and all the detail in the loudest partials is lost.
  float C = 0.16 * uSaturation * coh * (1.0 - 0.55 * mag);

  vec3 lab = vec3(L, C * cos(hue), C * sin(hue));
  vec3 rgb = oklabToLinearSrgb(lab);
  rgb += tr * mag * 1.2;                  // onset bloom
  return rgb;
}

// ---------------------------------------------------------------------------
// Mode 0 — waterfall, as a receding ridgeline landscape.
//
// A flat heatmap is the least interesting thing you can do with this data, and
// "waterfall" originally meant the 3D sonogram plot anyway. Each history frame
// is a ridgeline across log-frequency, displaced vertically by magnitude and
// pushed back toward a horizon. Rows are walked front to back and the first
// one that covers the pixel wins, which is painter's-algorithm hidden-surface
// removal — the near ridges genuinely occlude the far ones, and that occlusion
// is what reads as depth.
// ---------------------------------------------------------------------------
vec3 modeWaterfall(vec2 uv) {
  const int ROWS = 64;

  for (int i = 0; i < ROWS; i++) {
    float t = float(i) / float(ROWS - 1);   // 0 = nearest/newest, 1 = horizon

    // Non-linear so rows bunch up toward the horizon rather than marching back
    // at an even pace — even spacing looks like a staircase, not a distance.
    float persp = pow(t, 0.72);
    float baseY  = mix(0.03, 0.82, persp);
    float shrink = mix(1.0, 0.62, persp);   // far rows are narrower

    // This pixel's x, expressed in the row's own (narrowed) frequency space.
    float lx = (uv.x - 0.5) / shrink + 0.5;
    if (lx < 0.0 || lx > 1.0) continue;

    float age = t * uWindowFrac * float(uHistoryLen);
    vec4 s = sampleHistory(lx * float(uBins), age);

    // Displacement shrinks with distance so the perspective stays consistent.
    float ridge = baseY + s.r * 0.17 * shrink;
    if (uv.y > ridge) continue;             // above this crest — look further back

    // Inside this row's surface: it hides everything behind it.
    float below = smoothstep(0.0, 0.005, ridge - uv.y);
    vec3 c = spectralColor(s);
    // Hot rim exactly on the crest, dimmer fill beneath, so the lines read as
    // lines instead of the whole surface glowing into mush. The fill still has
    // to carry real colour — drop it too far and the landscape reads as bare
    // wireframe floating in black.
    vec3 col = mix(c * 2.1, c * 0.42, below);
    // Aerial haze toward the horizon.
    return col * mix(1.0, 0.62, persp);
  }
  return vec3(0.0);
}

// ---------------------------------------------------------------------------
// Mode 1 — radial mandala.
// Frequency on the radius, time sweeping angularly. Because the radius is
// log-frequency, octaves are evenly spaced rings and a harmonic stack shows up
// as concentric circles.
// ---------------------------------------------------------------------------
vec3 modeMandala(vec2 uv) {
  vec2 p = (uv * 2.0 - 1.0);
  p.x *= uResolution.x / uResolution.y;

  float r = length(p);
  float a = atan(p.y, p.x) / TAU + 0.5;   // 0..1 once around

  // Time runs around the full circle rather than being folded into a wedge.
  // Folding N-ways would squeeze the whole history into 1/N of the
  // circumference — well past the pixel Nyquist limit, which is what turns
  // the picture into chevron moiré instead of a mandala.
  float age = a * uWindowFrac * float(uHistoryLen);

  // Symmetry becomes a petal warp on the radius: all the flower geometry,
  // none of the undersampling.
  float petal = 1.0 + 0.055 * sin(a * TAU * uSymmetry);
  float rr = clamp(r * petal * 1.02, 0.0, 1.0);

  // Area on a disc grows as r², so a linear bin↦radius map hands the top
  // octaves — which are mostly cymbal wash — more than half the picture,
  // while the harmonically dense low end is crushed into the middle. Biasing
  // the exponent toward 2 equalizes screen area per octave.
  float bin = pow(rr, 1.6) * float(uBins);

  vec3 col = spectralColor(sampleHistory(bin, age));
  // Fade the very centre, where every angle collapses onto one texel.
  col *= smoothstep(0.0, 0.07, r);
  return col;
}

// ---------------------------------------------------------------------------
// Mode 2 — phase flow field.
// Rather than simulate particles, the UV is advected through the local
// spectral gradient a few times. Cheap, and it gives the same liquid smoke
// because the gradient of a spectrogram genuinely is the direction energy is
// travelling.
// ---------------------------------------------------------------------------
vec3 modeFlow(vec2 uv) {
  vec2 p = uv;
  vec3 acc = vec3(0.0);
  float wsum = 0.0;

  // Line-integral convolution. Sampling only at the end of the advection just
  // gives a wobbled spectrogram; smearing colour *along* the streamline is
  // what reads as smoke, because each pixel ends up showing the history of the
  // energy that flowed through it.
  const int STEPS = 8;
  float amp = 0.030 * (0.5 + uLevel);

  for (int i = 0; i < STEPS; i++) {
    float age = (1.0 - p.x) * uWindowFrac * float(uHistoryLen);
    float bin = p.y * float(uBins);
    vec4 c = sampleHistory(bin, age);

    float w = exp(-float(i) * 0.22);
    acc += spectralColor(c) * w;
    wsum += w;

    // Gradient of magnitude, rotated 90° so flow runs along contours rather
    // than straight up them — that is what makes it curl instead of smear.
    vec4 cb = sampleHistory(bin + 4.0, age);
    vec4 ca = sampleHistory(bin, age + 4.0);
    vec2 grad = vec2(ca.r - c.r, cb.r - c.r);
    vec2 flow = vec2(-grad.y, grad.x) * 2.5;

    // Pitch class steers the drift, so different notes move different ways.
    float ang = c.g * TAU;
    flow += 0.45 * vec2(cos(ang), sin(ang)) * c.b;

    p += flow * amp;
  }

  return acc / wsum * 1.5;
}

// ---------------------------------------------------------------------------
// Mode 3 — harmonic helix / chroma torus.
// Angle is pitch class and radius is octave, so every C sits on one spoke.
// A chord becomes a fixed geometric figure; a key change visibly rotates the
// entire structure.
// ---------------------------------------------------------------------------
vec3 modeHelix(vec2 uv) {
  vec2 p = (uv * 2.0 - 1.0);
  p.x *= uResolution.x / uResolution.y;

  float r = length(p);
  float a = atan(p.y, p.x) / TAU + 0.5;   // 0..1 around the circle

  // Angle is pitch class, radius is octave. One turn of the circle is exactly
  // one octave of the log-frequency axis, so a note and its octave sit on the
  // same spoke at different radii and a harmonic stack lines up radially.
  float bpo = float(uBinsPerOctave);
  float octaves = float(uBins) / bpo;
  float oct = clamp((r - 0.14) / 0.80, 0.0, 1.0) * octaves;

  // The fractional turn advances the octave continuously, which is what makes
  // it a helix rather than a stack of disconnected rings.
  float binF = (floor(oct) + a) * bpo;

  // Older material sits further out, so ascending lines spiral outward.
  float age = (oct / octaves) * uWindowFrac * float(uHistoryLen) * 0.5;

  vec4 s = sampleHistory(binF, age);
  vec3 col = spectralColor(s);
  col *= smoothstep(0.13, 0.19, r) * smoothstep(1.02, 0.90, r);
  return col;
}

vec3 renderMode(int mode, vec2 uv) {
  if (mode == 0) return modeWaterfall(uv);
  if (mode == 1) return modeMandala(uv);
  if (mode == 2) return modeFlow(uv);
  return modeHelix(uv);
}

void main() {
  vec2 uv = vUv;

  vec3 col = renderMode(uModeA, uv);
  if (uBlend > 0.001) {
    col = mix(col, renderMode(uModeB, uv), uBlend);
  }

  // Onset flash. Multiplicative, not additive: adding a flat term lifts the
  // silent background off black and the whole field reads as grey haze.
  float vign = 1.0 - 0.55 * length(uv - 0.5);
  col *= vign * (1.0 + uFlux * 0.55);

  fragColor = vec4(linearToSrgb(col), 1.0);
}
`;
