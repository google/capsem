// Distance-driven scale and lateral nudge, following Build UI's Magnified Dock.
// Anchors stay fixed; neither the pointer nor distance reads transformed images.
// https://buildui.com/recipes/magnified-dock
export const dockTarget = (distance: number, proximity: number) => {
  const influence = Math.max(0, 1 - Math.abs(distance) / 110) * proximity;
  return { scale: 1 + .7 * influence, x: -distance / 110 * 40 * influence };
};

export function spring(value: number, velocity: number, target: number, milliseconds: number) {
  // Substeps keep the original mass=.1 / stiffness=170 / damping=12 stable.
  let remaining = Math.min(milliseconds / 1000, .05);
  while (remaining > 0) {
    const dt = Math.min(remaining, 1 / 240);
    velocity += ((target - value) * 170 - velocity * 12) / .1 * dt;
    value += velocity * dt;
    remaining -= dt;
  }
  return { value, velocity };
}
