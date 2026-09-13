import React from 'react';
import logoLight from '@/assets/vox_128.png';
import logoDark from '@/assets/vox_128_dark.png';
import logo1024 from '@/assets/vox_1024.png';
import logo1024Dark from '@/assets/vox_1024_dark.png';
import logoExpandedLight from '@/assets/vox-expanded.png';
import logoExpandedDark from '@/assets/vox-expanded-dark.png';

export {
  logoLight as voxAppImage,
  logoLight as voxAppImage128,
  logoDark as voxAppImage128Dark,
  logo1024 as voxAppImage1024,
  logo1024Dark as voxAppImage1024Dark,
  logoExpandedLight as voxExpandedImage,
  logoExpandedDark as voxExpandedImageDark,
};

interface VoxLogoProps {
  className?: string;
  expanded?: boolean;
}

/**
 * Vox brand logo using the official brand asset PNGs.
 * Supports compact circular icon (default) or the expanded wordmark ("expanded" prop).
 * Switches between light and dark variants based on the document's
 * current dark-mode class (which Tauri's root `<html class="dark">` sets).
 */
export const VoxLogo: React.FC<VoxLogoProps> = ({ className = 'w-6 h-6', expanded = false }) => {
  const [isDark, setIsDark] = React.useState(
    () => document.documentElement.classList.contains('dark')
  );

  React.useEffect(() => {
    const observer = new MutationObserver(() => {
      setIsDark(document.documentElement.classList.contains('dark'));
    });
    observer.observe(document.documentElement, { attributeFilter: ['class'] });
    return () => observer.disconnect();
  }, []);

  const src = expanded
    ? (isDark ? logoExpandedDark : logoExpandedLight)
    : (isDark ? logoDark : logoLight);

  return (
    <img
      src={src}
      alt="Vox logo"
      className={`${className} object-contain`}
      draggable={false}
    />
  );
};
