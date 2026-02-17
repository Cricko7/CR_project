import { cn } from '../../lib/cn';

export interface SliderProps {
  value: number[];
  onValueChange: (value: number[]) => void;
  min?: number;
  max?: number;
  step?: number;
  className?: string;
  disabled?: boolean;
}

export const Slider = ({
  value,
  onValueChange,
  min = 0,
  max = 100,
  step = 1,
  className,
  disabled = false
}: SliderProps) => {
  const current = value[0] ?? min;
  const percent = ((current - min) / (max - min)) * 100;

  return (
    <div className={cn('relative w-full py-2', className)}>
      <div className="h-1.5 w-full rounded-full bg-white/15" />
      <div
        className="pointer-events-none absolute left-0 top-2 h-1.5 rounded-full bg-cyan-400"
        style={{ width: `${percent}%` }}
      />
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={current}
        disabled={disabled}
        onChange={(event) => onValueChange([Number(event.target.value)])}
        className={cn(
          'absolute inset-0 h-6 w-full cursor-pointer appearance-none bg-transparent',
          'disabled:cursor-not-allowed'
        )}
      />
    </div>
  );
};
