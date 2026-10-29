// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Material Symbols (Apache-2.0, weight 400, outlined), inlined at build
 * time. Only the names below are bundled; add a name here to use it.
 */
const SOURCES = import.meta.glob<string>(
  "/node_modules/@material-symbols/svg-400/outlined/{account_tree,attach_file,add,add_circle,upload,add_link,admin_panel_settings,arrow_downward,arrow_upward,bar_chart,calculate,calendar_month,calendar_today,check,check_box,check_box_outline_blank,check_circle,chevron_left,checklist,chevron_right,close,code,dashboard,data_object,database,delete,description,drag_indicator,draw,drive_file_move,edit,error,expand_circle_down,fact_check,folder_open,format_bold,format_h1,format_h2,format_h3,format_indent_decrease,format_indent_increase,format_ink_highlighter,format_italic,format_list_bulleted,format_list_numbered,format_quote,format_strikethrough,function,grid_on,grid_view,help,history,home,horizontal_rule,image,inbox,info,keyboard_arrow_down,left_panel_close,left_panel_open,lightbulb,linear_scale,link,menu,monitoring,more_horiz,more_vert,notes,notifications,open_in_new,palette,person,photo_library,polyline,schema,search,smart_display,star,star-fill,sticky_note_2,tab,table,table_view,timeline,toc,view_column_2,view_kanban,view_list,view_timeline,view_week,warning,add_task,comment,content_copy,share}.svg",
  { query: "?raw", import: "default", eager: true },
);

/** Pulls the single path out of a Material Symbols file. */
function pathOf(svg: string): string {
  const start = svg.indexOf(' d="');
  return start === -1 ? "" : svg.slice(start + 4, svg.indexOf('"', start + 4));
}

const PATHS: ReadonlyMap<string, string> = new Map(
  Object.entries(SOURCES).map(([file, svg]) => [
    file.slice(file.lastIndexOf("/") + 1, -".svg".length),
    pathOf(svg),
  ]),
);

/** Whether an icon name is bundled. */
export function hasIcon(name: string): boolean {
  return PATHS.has(name);
}

/** A decorative icon; the control that holds it carries the accessible name. */
export function Icon({ name, size = 24 }: { readonly name: string; readonly size?: number }) {
  return (
    <svg
      className="vl-icon"
      width={size}
      height={size}
      viewBox="0 -960 960 960"
      aria-hidden="true"
      focusable="false"
    >
      <path d={PATHS.get(name) ?? ""} />
    </svg>
  );
}
