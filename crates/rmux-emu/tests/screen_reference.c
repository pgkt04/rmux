/* Test-only driver for tmux screen.c and screen-write.c at 8f25579c.
 * No pane/server or image adapter is installed; unexpected adapter calls abort.
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
COMMAND(setselection) COMMAND(rawstring)
#undef COMMAND
static void hex(const char *s,size_t n);
static void cellout(const struct grid_cell *gc) {
 printf(" %u %u %d %d %d %u %u ",gc->attr,gc->flags,gc->fg,gc->bg,gc->us,gc->link,gc->data.width);hex(gc->data.data,gc->data.size);
}
/* Every tty command the writer emits, in order, with the old-state context. */
void tty_write(void (*cmd)(struct tty *,const struct tty_ctx *),struct tty_ctx *ctx) {
 const char *name=NULL;int n=0,cell=0,cells=0;
#define NAME(x,hasn,hascell,hascells) if(cmd==tty_cmd_##x){name=#x;n=hasn;cell=hascell;cells=hascells;}
 NAME(syncstart,0,0,0) NAME(cell,0,1,0) NAME(cells,0,1,1) NAME(redrawline,1,0,0)
 NAME(alignmenttest,0,0,0) NAME(insertcharacter,1,0,0) NAME(deletecharacter,1,0,0)
 NAME(clearcharacter,1,0,0) NAME(insertline,1,0,0) NAME(deleteline,1,0,0)
 NAME(clearendofscreen,0,0,0) NAME(clearstartofscreen,0,0,0) NAME(clearscreen,0,0,0)
 NAME(scrollup,1,0,0) NAME(scrolldown,1,0,0) NAME(reverseindex,0,0,0)
#undef NAME
 if(name==NULL)abort();
 printf("draw %s %u %u %u %u %u %d %d %d",name,ctx->ocx,ctx->ocy,ctx->orupper,ctx->orlower,ctx->bg,
  !!(ctx->flags&TTY_CTX_SYNC),!!(ctx->flags&TTY_CTX_WRAPPED),!!(ctx->flags&TTY_CTX_CELL_INVALIDATE));
 if(n)printf(" %u",ctx->n);
 if(cell)cellout(ctx->cell);
 if(cells){putchar(' ');hex(ctx->data.data,ctx->data.size);}
 puts("");
}
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

static void hex(const char *s,size_t n) {size_t i;for(i=0;i<n;i++)printf("%02x",(unsigned char)s[i]);}
static size_t unhex(const char *s,char *out) {unsigned int n;size_t len=0;while(*s && sscanf(s,"%2x",&n)==1){*out++=n;s+=2;len++;}*out=0;return len;}
static char *tok(void) {char *t=strtok(NULL," ");return t?t:"";}
static u_int num(void) {return (u_int)strtoul(tok(),NULL,10);}
static int snum(void) {return (int)strtol(tok(),NULL,10);}
static struct grid_cell cellspec(void) {
 struct grid_cell gc;char buf[UTF8_SIZE+1];memset(&gc,0,sizeof gc);
 gc.attr=num();gc.flags=num();gc.fg=snum();gc.bg=snum();gc.us=snum();gc.link=num();gc.data.width=num();
 gc.data.size=gc.data.have=unhex(tok(),buf);if(gc.data.size>UTF8_SIZE)abort();memcpy(gc.data.data,buf,gc.data.size);return gc;
}
static void dump_grid(struct grid *gd) {
 u_int y,x;struct grid_cell gc;
 printf("grid %u %u %u %u %u %u %u %u %d\n",gd->sx,gd->sy,gd->hsize,gd->hscrolled,gd->hlimit,gd->scroll_added,gd->scroll_collected,gd->scroll_generation,gd->flags);
 for(y=0;y<gd->hsize+gd->sy;y++) {
  struct grid_line *gl=&gd->linedata[y];struct osc133_data *od=&gl->osc133_data;
  printf("line %u %u %u %u %u %u osc=%u/%u/%u/%u/%u\n",y,gl->flags,gl->cellused,gl->cellsize,gl->extdsize,gl->time,od->prompt_col,od->cmd_col,od->out_start_col,od->out_end_col,od->exit_status);
  for(x=0;x<gl->cellsize;x++) {
   grid_get_cell(gd,x,y,&gc);
   printf("cell %u %u %u %d %d %d %u %u ",x,gc.attr,gc.flags,gc.fg,gc.bg,gc.us,gc.link,gc.data.width);hex(gc.data.data,gc.data.size);puts("");
  }
 }
}
static void dump(struct screen *s,u_int step) {
 printf("boundary %u\nscreen %u %u %u %u %u %u %u %d %d\n",step,s->cx,s->cy,s->rupper,s->rlower,s->mode,s->saved_cx,s->saved_cy,s->saved_flags,SCREEN_IS_ALTERNATE(s));
 dump_grid(s->grid);
 if(s->saved_grid != NULL || s->saved_cx != UINT_MAX) {
  struct grid_cell *gc=&s->saved_cell;
  printf("savedcell %u %u %d %d %d %u %u ",gc->attr,gc->flags,gc->fg,gc->bg,gc->us,gc->link,gc->data.width);hex(gc->data.data,gc->data.size);puts("");
 }
 if(s->saved_grid != NULL){puts("saved");dump_grid(s->saved_grid);}
}
int main(void) {
 char line[70000],bytes[32000],*op;struct screen s;struct screen_write_ctx ctx;struct grid_cell rendition;u_int step=0;int live=0,writing=0;
 setlocale(LC_CTYPE,"en_US.UTF-8");if(MB_CUR_MAX==1)setlocale(LC_CTYPE,"C.UTF-8");utf8_update_width_cache();
 memset(&s,0,sizeof s);memcpy(&rendition,&grid_default_cell,sizeof rendition);
 while(fgets(line,sizeof line,stdin)) {
  line[strcspn(line,"\n")]=0;op=strtok(line," ");if(!op)continue;
  if(!strcmp(op,"new")) {
   u_int w=num(),h=num(),limit=num();extended_keys=snum();selector_wide=snum();
   if(writing){screen_write_stop(&ctx);writing=0;}if(live)screen_free(&s);
   memset(&s,0,sizeof s);screen_init(&s,w,h,limit);live=1;memcpy(&rendition,&grid_default_cell,sizeof rendition);
  } else if(!strcmp(op,"begin")) {if(writing)abort();screen_write_start(&ctx,&s);writing=1;}
  else if(!strcmp(op,"end")) {if(!writing)abort();screen_write_collect_end(&ctx);dump(&s,step++);screen_write_stop(&ctx);writing=0;dump(&s,step++);}
  else if(!strcmp(op,"boundary")) {if(writing)screen_write_collect_end(&ctx);dump(&s,step++);}
  else if(!strcmp(op,"resize")) {u_int w=num(),h=num();int r=snum(),e=snum(),c=snum();if(writing)abort();screen_resize_cursor(&s,w,h,r,e,c);}
  else if(!strcmp(op,"reinit")) {if(writing)abort();screen_reinit(&s,snum());}
  else if(!strcmp(op,"on")) {if(writing)abort();screen_alternate_on(&s,&rendition,snum());}
  else if(!strcmp(op,"off")) {if(writing)abort();screen_alternate_off(&s,&rendition,snum());}
  else if(!strcmp(op,"rendition")) rendition=cellspec();
  else if(!strcmp(op,"history")) {if(snum())s.grid->flags|=GRID_HISTORY;else s.grid->flags&=~GRID_HISTORY;}
  else if(!strcmp(op,"osc")) {u_int y=num();struct grid_line *gl=grid_get_line(s.grid,s.grid->hsize+y);gl->flags|=num();gl->osc133_data.prompt_col=num();gl->osc133_data.cmd_col=num();gl->osc133_data.out_start_col=num();gl->osc133_data.out_end_col=num();gl->osc133_data.exit_status=num();}
  else if(!strcmp(op,"cell")) {struct grid_cell gc=cellspec();screen_write_collect_end(&ctx);screen_write_cell(&ctx,&gc);}
  else if(!strcmp(op,"add")) {struct grid_cell gc=cellspec();screen_write_collect_add(&ctx,&gc);}
  else if(!strcmp(op,"text")) {size_t i,n;screen_write_collect_end(&ctx);rendition=cellspec();n=unhex(tok(),bytes);for(i=0;i<n;i++){utf8_set(&rendition.data,bytes[i]);screen_write_collect_add(&ctx,&rendition);}}
  else {
   screen_write_collect_end(&ctx);
   if(!strcmp(op,"move")){int x=snum(),y=snum(),o=snum();screen_write_cursormove(&ctx,x,y,o);}
   else if(!strcmp(op,"up"))screen_write_cursorup(&ctx,num());
   else if(!strcmp(op,"down"))screen_write_cursordown(&ctx,num());
   else if(!strcmp(op,"right"))screen_write_cursorright(&ctx,num());
   else if(!strcmp(op,"left"))screen_write_cursorleft(&ctx,num());
   else if(!strcmp(op,"bs"))screen_write_backspace(&ctx);
   else if(!strcmp(op,"cr"))screen_write_carriagereturn(&ctx);
   else if(!strcmp(op,"region")){u_int a=num(),b=num();screen_write_scrollregion(&ctx,a,b);}
   else if(!strcmp(op,"lf")){int w=snum();u_int bg=num();screen_write_linefeed(&ctx,w,bg);}
   else if(!strcmp(op,"su")){u_int n=num(),bg=num();screen_write_scrollup(&ctx,n,bg);}
   else if(!strcmp(op,"sd")){u_int n=num(),bg=num();screen_write_scrolldown(&ctx,n,bg);}
   else if(!strcmp(op,"ri"))screen_write_reverseindex(&ctx,num());
   else if(!strcmp(op,"ich")){u_int n=num(),bg=num();screen_write_insertcharacter(&ctx,n,bg);}
   else if(!strcmp(op,"dch")){u_int n=num(),bg=num();screen_write_deletecharacter(&ctx,n,bg);}
   else if(!strcmp(op,"ech")){u_int n=num(),bg=num();screen_write_clearcharacter(&ctx,n,bg);}
   else if(!strcmp(op,"il")){u_int n=num(),bg=num();screen_write_insertline(&ctx,n,bg);}
   else if(!strcmp(op,"dl")){u_int n=num(),bg=num();screen_write_deleteline(&ctx,n,bg);}
   else if(!strcmp(op,"el"))screen_write_clearline(&ctx,num());
   else if(!strcmp(op,"el0"))screen_write_clearendofline(&ctx,num());
   else if(!strcmp(op,"el1"))screen_write_clearstartofline(&ctx,num());
   else if(!strcmp(op,"ed0"))screen_write_clearendofscreen(&ctx,num());
   else if(!strcmp(op,"ed1"))screen_write_clearstartofscreen(&ctx,num());
   else if(!strcmp(op,"ed2"))screen_write_clearscreen(&ctx,num());
   else if(!strcmp(op,"histclear"))screen_write_clearhistory(&ctx);
   else if(!strcmp(op,"align"))screen_write_alignmenttest(&ctx);
   else if(!strcmp(op,"reset"))screen_write_reset(&ctx);
   else if(!strcmp(op,"setmode"))screen_write_mode_set(&ctx,num());
   else if(!strcmp(op,"clearmode"))screen_write_mode_clear(&ctx,num());
   else {fprintf(stderr,"unknown operation: %s\n",op);abort();}
  }
 }
 if(writing)screen_write_stop(&ctx);if(live)screen_free(&s);return 0;
}
