<script lang="ts">
  import { resolveUsageRingTheme } from './usageRingTheme';
  import type { UsageRingTheme } from './usageRingTheme';

  export let ratio = 0;
  export let accent = '#5a6680';
  export let label = '';
  export let theme: UsageRingTheme | undefined = undefined;

  function normalizeRatio(value: number): number {
    if (!Number.isFinite(value)) return 0;
    return Math.min(1, Math.max(0, value));
  }

  function polarPoint(angleDegrees: number, radius: number) {
    const angleRadians = ((angleDegrees - 90) * Math.PI) / 180;
    return {
      x: 50 + radius * Math.cos(angleRadians),
      y: 50 + radius * Math.sin(angleRadians),
    };
  }

  function describeArc(radius: number, ratioValue: number): string {
    if (ratioValue <= 0) return '';

    // Keep the arc just shy of 360 degrees; SVG arc commands cannot draw a
    // mathematically complete circle as a single open path.
    const sweepRatio = Math.min(ratioValue, 0.9999);
    const start = polarPoint(0, radius);
    const end = polarPoint(sweepRatio * 360, radius);
    const largeArcFlag = sweepRatio > 0.5 ? 1 : 0;

    return `M ${start.x} ${start.y} A ${radius} ${radius} 0 ${largeArcFlag} 1 ${end.x} ${end.y}`;
  }

  $: resolvedTheme = resolveUsageRingTheme(theme, accent);
  $: normalizedRatio = normalizeRatio(ratio);
  $: radius = 50 - resolvedTheme.thickness / 2;
  $: innerRadius = Math.max(0, 50 - resolvedTheme.thickness);
  $: progressPath = describeArc(radius, normalizedRatio);
  $: cssVars = [
    `--ring-size:${resolvedTheme.size}px`,
    `--ring-label-color:${resolvedTheme.labelColor}`,
    `--ring-label-size:${resolvedTheme.labelSize}`,
    `--ring-label-weight:${resolvedTheme.labelWeight}`,
  ].join(';');
</script>

<div class="usage-ring" style={cssVars}>
  <svg class="ring-svg" viewBox="0 0 100 100" aria-hidden="true">
    {#if resolvedTheme.innerFill !== 'transparent'}
      <circle cx="50" cy="50" r={innerRadius} fill={resolvedTheme.innerFill} />
    {/if}

    <circle
      class="ring-track"
      cx="50"
      cy="50"
      r={radius}
      fill="none"
      stroke={resolvedTheme.trackColor}
      stroke-width={resolvedTheme.thickness}
    />

    {#if normalizedRatio >= 0.9999}
      <circle
        class="ring-progress"
        cx="50"
        cy="50"
        r={radius}
        fill="none"
        stroke={resolvedTheme.accentColor}
        stroke-width={resolvedTheme.thickness}
      />
    {:else if normalizedRatio > 0}
      <path
        class="ring-progress"
        d={progressPath}
        fill="none"
        stroke={resolvedTheme.accentColor}
        stroke-width={resolvedTheme.thickness}
        stroke-linecap="round"
      />
    {/if}
  </svg>

  {#if label}
    <span class="ring-label">{label}</span>
  {/if}
</div>

<style>
  .usage-ring {
    position: relative;
    display: grid;
    place-items: center;
    width: var(--ring-size);
    height: var(--ring-size);
    border-radius: 50%;
  }

  .ring-svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    overflow: visible;
  }

  .ring-track,
  .ring-progress {
    shape-rendering: geometricPrecision;
    vector-effect: non-scaling-stroke;
  }

  .ring-label {
    position: relative;
    z-index: 1;
    font-size: var(--ring-label-size);
    font-weight: var(--ring-label-weight);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--ring-label-color);
  }
</style>
