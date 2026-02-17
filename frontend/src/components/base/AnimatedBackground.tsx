import { cn } from '../../lib/cn';

export interface AnimatedBackgroundProps {
  className?: string;
}

const BLOBS = [
  {
    className:
      '-top-44 -left-36 h-[28rem] w-[28rem] bg-cyan-400/40 animate-aurora-one'
  },
  {
    className:
      'top-1/4 -right-32 h-[24rem] w-[24rem] bg-indigo-500/42 animate-aurora-two'
  },
  {
    className:
      '-bottom-6 left-1/3 h-[25rem] w-[25rem] bg-fuchsia-500/36 animate-aurora-three'
  },
  {
    className:
      '-bottom-24 right-1/4 h-[22rem] w-[22rem] bg-emerald-400/32 animate-aurora-four'
  }
] as const;

export const AnimatedBackground = ({ className }: AnimatedBackgroundProps) => (
  <div
    aria-hidden="true"
    className={cn('pointer-events-none absolute inset-0 -z-10 overflow-hidden', className)}
  >
    <div className="absolute inset-0 bg-slate-950/70" />
    {BLOBS.map((blob) => (
      <div
        key={blob.className}
        className={cn('absolute rounded-full blur-[85px]', blob.className)}
      />
    ))}
    <div className="noise-texture absolute inset-0 opacity-35 mix-blend-soft-light" />
  </div>
);
