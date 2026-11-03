use makepad_widgets::*;

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*

    mod.widgets.RobrixTabCloseButton = TabCloseButton {
        height: 10.0
        width: 10.0
        margin: Inset{ right: theme.space_2, left: -1 }
        draw_button +: {
            color: (RBX_FG_SECONDARY)
            color_hover: (RBX_DANGER_FG)
            color_active: COLOR_PRIMARY
        }

        animator: Animator{
            hover: {
                default: @off
                off: AnimatorState{
                    from: {all: Forward {duration: 0.1}}
                    apply: {
                        draw_button: {hover: 0.0}
                    }
                }

                on: AnimatorState{
                    cursor: MouseCursor.Hand
                    from: {all: Snap}
                    apply: {
                        draw_button: {hover: 1.0}
                    }
                }
            }
        }
    }

    mod.widgets.RobrixTab = Tab {
        width: Fit
        height: Fill

        align: Align{x: 0.0, y: 0.5}
        padding: 9
        // 1px bottom inset so the tab bar's bottom border line runs
        // continuously beneath every tab (including the active one).
        margin: Inset{bottom: 1}

        close_button: mod.widgets.RobrixTabCloseButton {}
        draw_text +: {
            text_style: theme.font_regular {}

            // Unified palette: dark text on the neutral unselected tab, white on the
            // teal selected tab. (No more orange hover text.)
            color: (RBX_FG_PRIMARY)
            color_hover: (RBX_FG_PRIMARY)
            color_active: (COLOR_PRIMARY)
        }

        draw_bg +: {
            // Unselected tabs: subtle neutral surface. Selected tab: the teal
            // accent (RBX_ACCENT), the unified UI selection color. Flat fills
            // (no vertical gradient).
            color: (RBX_BG_SURFACE_SUBTLE)
            color_2: (RBX_BG_SURFACE_SUBTLE)
            color_hover: (RBX_BG_HOVER)
            color_2_hover: (RBX_BG_HOVER)
            color_active: (RBX_ACCENT)
            color_2_active: (RBX_ACCENT)
            // Remove the border and rounded corners from the default Tab style
            border_size: 0.0
            border_radius: 3.0
        }

        animator: Animator{
            hover: {
                default: @off
                off: AnimatorState{
                    from: {all: Forward {duration: 0.2}}
                    apply: {
                        draw_bg: {hover: 0.0}
                        draw_text: {hover: 0.0}
                    }
                }

                on: AnimatorState{
                    cursor: MouseCursor.Hand
                    from: {all: Forward {duration: 0.1}}
                    apply: {
                        draw_bg: {hover: snap(1.0)}
                        draw_text: {hover: snap(1.0)}
                    }
                }
            }

            active: {
                default: @off
                off: AnimatorState{
                    from: {all: Forward {duration: 0.3}}
                    apply: {
                        close_button: {draw_button: {active: 0.0}}
                        draw_bg: {active: 0.0}
                        draw_text: {active: 0.0}
                    }
                }

                on: AnimatorState{
                    from: {all: Snap}
                    apply: {
                        close_button: {draw_button: {active: 1.0}}
                        draw_bg: {active: 1.0}
                        draw_text: {active: 1.0}
                    }
                }
            }
        }
    }

    mod.widgets.RobrixTabBar = TabBar {
        CloseableTab := mod.widgets.RobrixTab {closeable: true}
        PermanentTab := mod.widgets.RobrixTab {closeable: false}

        draw_drag +: {
            draw_depth: 10
            color: #x0
        }
        // Both layers are drawn FLAT with a 1px bottom border, replacing the
        // theme's built-in bottom "shadow". draw_bg covers the whole bar;
        // draw_fill covers the area after the last tab, so both need the same
        // fill + border for a seamless bar.
        draw_fill +: {
            color: (RBX_BG_SURFACE_SUBTLE)
            border_color: (RBX_STROKE_STRONG)
            pixel: fn() {
                let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                sdf.rect(0., 0., self.rect_size.x, self.rect_size.y)
                sdf.fill(self.color)
                sdf.rect(0., self.rect_size.y - 1.0, self.rect_size.x, 1.0)
                sdf.fill(self.border_color)
                return sdf.result
            }
        }
        draw_bg +: {
            color: (RBX_BG_SURFACE_SUBTLE)
            border_color: (RBX_STROKE_STRONG)
            pixel: fn() {
                let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                sdf.rect(0., 0., self.rect_size.x, self.rect_size.y)
                sdf.fill(self.color)
                sdf.rect(0., self.rect_size.y - 1.0, self.rect_size.x, 1.0)
                sdf.fill(self.border_color)
                return sdf.result
            }
        }

        width: Fill
        height: max(theme.tab_height, 25.)

        scroll_bars: ScrollBarsTabs {
            show_scroll_x: true
            show_scroll_y: false
            scroll_bar_x +: {
                bar_size: 4
                use_vertical_finger_scroll: true
            }
        }
    }

    mod.widgets.RobrixDock = Dock {
        flow: Down

        round_corner +: {
            color: COLOR_SECONDARY
        }

        padding: Inset{left: theme.dock_border_size, top: 0, right: theme.dock_border_size, bottom: theme.dock_border_size}
        drag_target_preview +: {
            draw_depth: 10.0
            color: mix(COLOR_ACTIVE_PRIMARY, #FFFFFF00, 0.5)
        }
        tab_bar: mod.widgets.RobrixTabBar {}
        splitter: mod.widgets.RobrixSplitter {}
    }
}
