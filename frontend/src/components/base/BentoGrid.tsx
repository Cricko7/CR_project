import type { HTMLAttributes } from 'react';
import { cn } from '../../lib/cn';

export interface BentoGridProps extends HTMLAttributes<HTMLDivElement> {}

export type BentoSpan = 1 | 2;

export interface BentoGridItemProps extends HTMLAttributes<HTMLDivElement> {
  span?: BentoSpan;
}

export const BentoGrid = ({ className, ...props }: BentoGridProps) => (
  <div className={cn('grid grid-cols-1 gap-6 lg:grid-cols-3', className)} {...props} />
);

export const BentoGridItem = ({
  className,
  span = 1,
  ...props
}: BentoGridItemProps) => (
  <div className={cn(span === 2 ? 'lg:col-span-2' : 'lg:col-span-1', className)} {...props} />
);
