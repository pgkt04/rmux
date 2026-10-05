#include <stddef.h>
#include "tmux.h"

#define SIZE(type) printf(#type " sizeof=%zu\n", sizeof(struct type))
#define OFFSET(type, field) printf(#type "." #field "=%zu\n", offsetof(struct type, field))
int main(void) {
    SIZE(grid_line);
    OFFSET(grid_line, celldata); OFFSET(grid_line, extddata);
    OFFSET(grid_line, cellused); OFFSET(grid_line, cellsize);
    OFFSET(grid_line, extdsize); OFFSET(grid_line, time);
    OFFSET(grid_line, osc133_data); OFFSET(grid_line, flags);
    SIZE(grid_cell);
    OFFSET(grid_cell, data); OFFSET(grid_cell, attr); OFFSET(grid_cell, flags);
    OFFSET(grid_cell, fg); OFFSET(grid_cell, bg); OFFSET(grid_cell, us); OFFSET(grid_cell, link);
    SIZE(grid_cell_entry);
    OFFSET(grid_cell_entry, offset); OFFSET(grid_cell_entry, data); OFFSET(grid_cell_entry, flags);
    SIZE(grid_extd_entry);
    OFFSET(grid_extd_entry, data); OFFSET(grid_extd_entry, attr); OFFSET(grid_extd_entry, flags);
    OFFSET(grid_extd_entry, fg); OFFSET(grid_extd_entry, bg); OFFSET(grid_extd_entry, us); OFFSET(grid_extd_entry, link);
    return 0;
}
