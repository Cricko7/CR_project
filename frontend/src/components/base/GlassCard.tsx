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
  glowColor = 'rgba(125, 211, 252, 0.16)',
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
        'shadow-[inset_0_1px_0_rgba(255,255,255,0.1)]',
        className
      )}
      animate={{
        scale: hover && isHovered ? 1.01 : 1,
        y: hover && isHovered ? -2 : 0
      }}
      transition={transition ?? { type: 'spring', stiffness: 230, damping: 22, mass: 0.9 }}
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
        transition={{ duration: 0.28, ease: 'easeOut' }}
        style={{ boxShadow: `inset 0 0 72px ${glowColor}` }}
      />
      <motion.div className="relative z-[1]">{children}</motion.div>
    </motion.div>
  );
};
