import { cn } from '../../lib/cn';
import { GlassCard, type GlassCardProps } from './GlassCard';

export interface SkeletonCardProps extends Omit<GlassCardProps, 'children'> {
  lines?: number;
  showAvatar?: boolean;
}

export const SkeletonCard = ({
  className,
  lines = 3,
  showAvatar = true,
  ...props
}: SkeletonCardProps) => (
  <GlassCard
    hover={false}
    aria-busy="true"
    className={cn('skeleton-shimmer p-6', className)}
    {...props}
  >
    <div className="animate-pulse space-y-4">
      {showAvatar ? (
        <div className="flex items-center gap-4">
          <div className="h-12 w-12 rounded-2xl bg-white/10" />
          <div className="space-y-2">
            <div className="h-4 w-32 rounded-full bg-white/10" />
            <div className="h-3 w-20 rounded-full bg-white/10" />
          </div>
        </div>
      ) : null}
      {Array.from({ length: lines }).map((_, index) => (
        <div
          key={`line-${index + 1}`}
          className={cn(
            'h-3 rounded-full bg-white/10',
            index === lines - 1 ? 'w-2/3' : 'w-full'
          )}
        />
      ))}
    </div>
  </GlassCard>
);
