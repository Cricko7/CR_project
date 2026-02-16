import { cn } from '../../lib/cn';

export interface AnimatedBackgroundProps {
  className?: string;
}

const BLOBS = [
  {
    className:
      '-top-36 -left-28 h-[24rem] w-[24rem] bg-cyan-400/30 animate-aurora-one'
  },
  {
    className:
      'top-1/4 -right-24 h-[20rem] w-[20rem] bg-indigo-500/30 animate-aurora-two'
  },
  {
    className:
      'bottom-0 left-1/3 h-[22rem] w-[22rem] bg-fuchsia-500/25 animate-aurora-three'
  },
  {
    className:
      '-bottom-20 right-1/4 h-[18rem] w-[18rem] bg-emerald-400/20 animate-aurora-four'
  }
] as const;

export const AnimatedBackground = ({ className }: AnimatedBackgroundProps) => (
  <div
    aria-hidden="true"
    className={cn('pointer-events-none absolute inset-0 -z-10 overflow-hidden', className)}
  >
    <div className="absolute inset-0 bg-slate-950/45" />
    {BLOBS.map((blob) => (
      <div
        key={blob.className}
        className={cn('absolute rounded-full blur-[100px]', blob.className)}
      />
    ))}
    <div className="noise-texture absolute inset-0 opacity-25 mix-blend-soft-light" />
  </div>
);
