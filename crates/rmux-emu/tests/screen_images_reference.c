/* Ported from tmux screen.c, screen-write.c, image.c and image-sixel.c @ 8f25579c.
 * Test-only image hook driver; unexpected server adapter calls abort.
 */
#include <locale.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "tmux.h"
#include "grid.c"
#include "grid-view.c"
#include "colour.c"
#include "hyperlinks.c"
#include "screen.c"
#include "screen-write.c"
#include "image-sixel.c"
#include "image.c"

struct options *global_options;
struct timeval start_time;
time_t current_time;
static int extended_keys, selector_wide;

void log_debug(const char *fmt, ...) {(void)fmt;}
int log_get_level(void) {return 0;}
__dead void fatalx(const char *fmt, ...) {(void)fmt;abort();}
__dead void fatal(const char *fmt, ...) {(void)fmt;abort();}
long long options_get_number(struct options *o, const char *n) {
 (void)o;
 if (!strcmp(n,"extended-keys")) return extended_keys ? 2 : 0;
 if (!strcmp(n,"variation-selector-always-wide")) return selector_wide;
 abort();
}
struct options_entry *options_get(struct options *o,const char *n) {(void)o;(void)n;return NULL;}
struct options_array_item *options_array_first(struct options_entry *o) {(void)o;return NULL;}
struct options_array_item *options_array_next(struct options_array_item *o) {(void)o;return NULL;}
union options_value *options_array_item_value(struct options_array_item *o) {(void)o;return NULL;}
union options_value *options_array_getv(struct options_entry *o,const char *fmt,...) {(void)o;(void)fmt;return NULL;}
const struct options_table_entry *options_table_entry(struct options_entry *o) {(void)o;abort();}
const char *options_get_string(struct options *o,const char *n) {(void)o;(void)n;abort();}
struct style *options_string_to_style(struct options *o,const char *n,struct format_tree *f) {(void)o;(void)n;(void)f;return NULL;}

/* Function identities are retained by the unmodified writer. */
#define COMMAND(name) void tty_cmd_##name(struct tty *t,const struct tty_ctx *c) {(void)t;(void)c;abort();}
COMMAND(syncstart) COMMAND(cell) COMMAND(cells) COMMAND(redrawline)
COMMAND(alignmenttest) COMMAND(insertcharacter) COMMAND(deletecharacter)
COMMAND(clearcharacter) COMMAND(insertline) COMMAND(deleteline)
COMMAND(clearendofscreen) COMMAND(clearstartofscreen) COMMAND(clearscreen)
COMMAND(scrollup) COMMAND(scrolldown) COMMAND(reverseindex)
COMMAND(setselection) COMMAND(rawstring) COMMAND(sixelimage)
#undef COMMAND
void tty_write(void (*cmd)(struct tty *,const struct tty_ctx *),struct tty_ctx *ctx) {(void)cmd;(void)ctx;}
struct visible_ranges *window_visible_ranges(struct window_pane *wp,int x,int y,u_int n,struct visible_ranges *out) {
 static struct visible_range span;
 static struct visible_ranges ranges;
 (void)y;(void)out;
 if(wp != NULL || x < 0) abort();
 span.px=x;span.nx=n;ranges.ranges=&span;ranges.used=1;ranges.size=1;
 return &ranges;
}
int window_position_is_visible(struct visible_ranges *r,u_int x) {
 u_int i;for(i=0;i<r->used;i++)if(x>=r->ranges[i].px && x<r->ranges[i].px+r->ranges[i].nx)return 1;return 0;
}
void tty_update_window_offset(struct window *w) {(void)w;abort();}
int tty_window_offset(struct tty *t,u_int *a,u_int *b,u_int *c,u_int *d) {(void)t;(void)a;(void)b;(void)c;(void)d;abort();}
void tty_default_colours(struct grid_cell *c,struct window_pane *w,u_int *d) {(void)c;(void)w;(void)d;abort();}
int status_at_line(struct client *c) {(void)c;abort();}
u_int status_line_size(struct client *c) {(void)c;abort();}
int session_has(struct session *s,struct window *w) {(void)s;(void)w;abort();}
void redraw_damage_window(struct window *w,u_int x,u_int y,u_int n,u_int m) {(void)w;(void)x;(void)y;(void)n;(void)m;abort();}
int window_pane_floating_overlaps(struct window_pane *a,struct window_pane *b) {(void)a;(void)b;abort();}
int window_pane_scrollbar_overlay_visible(struct window_pane *w) {(void)w;abort();}
void window_pane_scrollbar_redraw(struct window_pane *w) {(void)w;abort();}
u_int menu_x(struct menu_data *m) {(void)m;abort();}
u_int menu_y(struct menu_data *m) {(void)m;abort();}
u_int menu_width(struct menu_data *m) {(void)m;abort();}
u_int menu_height(struct menu_data *m) {(void)m;abort();}
void window_pane_clear_resizes(struct window_pane *w,struct window_pane_resize *r) {(void)w;(void)r;abort();}
void window_pane_send_resize(struct window_pane *w,u_int x,u_int y) {(void)w;(void)x;(void)y;abort();}
void layout_fix_panes(struct window *w,struct window_pane *p) {(void)w;(void)p;abort();}
void server_redraw_window_borders(struct window *w) {(void)w;abort();}


static struct sixel_image *make_image(u_int x,u_int y) {
 char payload[128];int n=snprintf(payload,sizeof payload,"q\"1;1;%u;%u#0",x,y);
 return sixel_parse(payload,n,0,1,1);
}
static void dump_images(struct screen *s) {
 struct image *im;
 printf("%u %u",s->cx,s->cy);
 TAILQ_FOREACH(im,&s->images,entry) {
  size_t n=0,i;char *encoded=sixel_print(im->data,NULL,&n);
  printf(" %u,%u,%u,%u:",im->px,im->py,im->sx,im->sy);
  if(encoded) {for(i=0;i<n;i++)printf("%02x",(unsigned char)encoded[i]);free(encoded);}
 }
 putchar('\n');
}
int main(void) {
 struct screen s;struct screen_write_ctx ctx;struct grid_cell gc;
 char line[256],*op;u_int px,py,sx,sy,width,height,x,y,upper,lower;int fill;
 while(fgets(line,sizeof line,stdin)) {
  if(sscanf(line,"%u %u %u %u %u %u %u %u %u %u %d",&width,&height,&px,&py,&sx,&sy,&x,&y,&upper,&lower,&fill)!=11)abort();
  op=strrchr(line,' ')+1;
  memset(&s,0,sizeof s);screen_init(&s,width,height,20);s.cx=px;s.cy=py;
  if(sx&&sy&&strncmp(op,"image",5)) image_store(&s,make_image(sx,sy));
  memcpy(&gc,&grid_default_cell,sizeof gc);utf8_set(&gc.data,'A');
  if(fill) grid_view_set_cell(s.grid,0,y,&gc);
  s.cx=x;s.cy=y;s.rupper=upper;s.rlower=lower;
  if(!strncmp(op,"resize",6)) {screen_resize_cursor(&s,width,height,0,1,1);dump_images(&s);screen_free(&s);continue;}
  if(!strncmp(op,"alternate",9) || !strncmp(op,"reset",5)) {
   screen_alternate_on(&s,&gc,0);s.cx=0;s.cy=0;image_store(&s,make_image(1,1));
   if(!strncmp(op,"reset",5)) screen_reinit(&s,0);else screen_alternate_off(&s,&gc,0);
   dump_images(&s);screen_free(&s);continue;
  }
  screen_write_start(&ctx,&s);
  if(!strncmp(op,"alignment",9)) screen_write_alignmenttest(&ctx);
  else if(!strncmp(op,"insert-character",16)) screen_write_insertcharacter(&ctx,1,8);
  else if(!strncmp(op,"delete-character",16)) screen_write_deletecharacter(&ctx,1,8);
  else if(!strncmp(op,"clear-character",15)) screen_write_clearcharacter(&ctx,1,8);
  else if(!strncmp(op,"insert-line",11)) screen_write_insertline(&ctx,1,8);
  else if(!strncmp(op,"delete-line",11)) screen_write_deleteline(&ctx,1,8);
  else if(!strncmp(op,"clear-end-line",14)) screen_write_clearendofline(&ctx,8);
  else if(!strncmp(op,"clear-start-line",16)) screen_write_clearstartofline(&ctx,8);
  else if(!strncmp(op,"clear-line",10)) screen_write_clearline(&ctx,8);
  else if(!strncmp(op,"reverse",7)) screen_write_reverseindex(&ctx,8);
  else if(!strncmp(op,"linefeed",8)) screen_write_linefeed(&ctx,0,8);
  else if(!strncmp(op,"scroll-up",9)) screen_write_scrollup(&ctx,1,8);
  else if(!strncmp(op,"scroll-down",11)) screen_write_scrolldown(&ctx,1,8);
  else if(!strncmp(op,"clear-end-screen",16)) screen_write_clearendofscreen(&ctx,8);
  else if(!strncmp(op,"clear-start-screen",18)) screen_write_clearstartofscreen(&ctx,8);
  else if(!strncmp(op,"clear-screen",12)) screen_write_clearscreen(&ctx,8);
  else if(!strncmp(op,"text",4)) {screen_write_collect_add(&ctx,&gc);screen_write_collect_end(&ctx);}
  else if(!strncmp(op,"image",5)) screen_write_sixelimage(&ctx,make_image(sx,sy),8);
  else abort();
  screen_write_stop(&ctx);dump_images(&s);screen_free(&s);
 }
 return 0;
}
