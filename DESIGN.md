# Kiri interface system

The Kiri launch experience is a restrained, compact operational surface derived from Recordly's floating launch-window composition. It uses quiet panels, 34–38 px controls, tight 8/12/16/24 px spacing, 12–16 px radii, 1 px borders, and a single consistent Windows UI sans stack.

Color is expressed exclusively through the semantic tokens in `apps/desktop/src/styles/tokens.css`. The primary gradient is reserved for project creation and, in later phases, record/export and AI progress. Focus uses a high-contrast two-layer ring. Motion communicates popover/state changes and is disabled for reduced-motion users.

The launch shell has a slim title row, a dominant two-column creation area, compact secondary actions, and a recent-project list. The source-selector and controller are independent route/window boundaries and preserve Recordly's focused floating geometry.
