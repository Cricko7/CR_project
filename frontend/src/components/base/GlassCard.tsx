import { useState } from 'react';
import { motion, type HTMLMotionProps } from 'framer-motion';
import { cn } from '../../lib/cn';

export interface GlassCardProps extends HTMLMotionProps<'div'> {
  hover?: boolean;
  glowColor?: string;
}

export const GlassCard = ({
  children,
  className,
  hover = true,
  glowColor = 'rgba(56, 189, 248, 0.26)',
  onHoverEnd,
  onHoverStart,
  transition,
  ...props
}: GlassCardProps) => {
  const [isHovered, setIsHovered] = useState(false);

  return (
    <motion.div
      className={cn(
        'relative overflow-hidden rounded-3xl border border-white/10 bg-white/5 backdrop-blur-xl',
        'shadow-[inset_0_1px_0_rgba(255,255,255,0.12),0_8px_40px_rgba(2,6,23,0.28)]',
        className
      )}
      animate={{
        scale: hover && isHovered ? 1.018 : 1,
        y: hover && isHovered ? -5 : 0
      }}
      transition={transition ?? { type: 'spring', stiffness: 210, damping: 20, mass: 0.82 }}
      onHoverStart={(event, info) => {
        setIsHovered(true);
        onHoverStart?.(event, info);
      }}
      onHoverEnd={(event, info) => {
        setIsHovered(false);
        onHoverEnd?.(event, info);
      }}
      {...props}
    >
      <motion.div
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 rounded-[inherit]"
        animate={{ opacity: hover && isHovered ? 1 : 0 }}
        transition={{ duration: 0.24, ease: 'easeOut' }}
        style={{ boxShadow: `inset 0 0 96px ${glowColor}` }}
      />
      <motion.div className="relative z-[1]">{children}</motion.div>
    </motion.div>
  );
};
