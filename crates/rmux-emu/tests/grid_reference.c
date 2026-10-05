/* Test driver for unmodified tmux grid sources at 8f25579c. */
#include <locale.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "tmux.h"
#include "grid.c"
#include "grid-view.c"
#include "grid-reader.c"
#include "colour.c"
#include "hyperlinks.c"
struct options *global_options;
struct timeval start_time;
time_t current_time;
void log_debug(const char *fmt, ...) {(void)fmt;}
__dead void fatalx(const char *fmt, ...) {(void)fmt;abort();}
__dead void fatal(const char *fmt, ...) {(void)fmt;abort();}
struct options_entry *options_get(struct options *o,const char *n) {(void)o;(void)n;return NULL;}
struct options_array_item *options_array_first(struct options_entry *o) {(void)o;return NULL;}
struct options_array_item *options_array_next(struct options_array_item *o) {(void)o;return NULL;}
union options_value *options_array_item_value(struct options_array_item *o) {(void)o;return NULL;}
union options_value *options_array_getv(struct options_entry *o,const char *fmt,...) {(void)o;(void)fmt;return NULL;}
const struct options_table_entry *options_table_entry(struct options_entry *o) {(void)o;abort();}
const char *options_get_string(struct options *o,const char *n) {(void)o;(void)n;abort();}
struct style *options_string_to_style(struct options *o,const char *n,struct format_tree *f) {(void)o;(void)n;(void)f;return NULL;}
static void hex(const char *s,size_t n) {size_t i;for(i=0;i<n;i++)printf("%02x",(unsigned char)s[i]);}
static size_t unhex(const char *s,char *out) {unsigned int n;size_t len=0;if(s==NULL){*out=0;return 0;}while(*s && sscanf(s,"%2x",&n)==1){*out++=n;s+=2;len++;}*out=0;return len;}
static char *tok(void) {char *t=strtok(NULL," ");return t?t:"";}
static u_int num(void) {return (u_int)strtoul(tok(),NULL,10);}
static int snum(void) {return (int)strtol(tok(),NULL,10);}
static struct grid_cell cellspec(void) {
 struct grid_cell gc;char buf[64];memset(&gc,0,sizeof gc);
 gc.attr=num();gc.flags=num();gc.fg=snum();gc.bg=snum();gc.us=snum();gc.link=num();gc.data.width=num();
 gc.data.size=gc.data.have=unhex(tok(),buf);memcpy(gc.data.data,buf,gc.data.size);return gc;
}
static void dump(struct grid *gd) {
 u_int yy,xx;struct grid_cell gc;
 printf("grid %u %u %u %u %u %u %u %u %d\n",gd->sx,gd->sy,gd->hsize,gd->hscrolled,gd->hlimit,gd->scroll_added,gd->scroll_collected,gd->scroll_generation,gd->flags);
 for(yy=0;yy<gd->hsize+gd->sy;yy++){
  struct grid_line *gl=&gd->linedata[yy];
  printf("line %u flags=%u used=%u size=%u extd=%u time=%u\n",yy,gl->flags,gl->cellused,gl->cellsize,gl->extdsize,gl->time);
  printf("osc %u %u %u %u %u\n",gl->osc133_data.prompt_col,gl->osc133_data.cmd_col,gl->osc133_data.out_start_col,gl->osc133_data.out_end_col,gl->osc133_data.exit_status);
  for(xx=0;xx<gl->cellsize;xx++){
   struct grid_cell_entry *e=&gl->celldata[xx];
   grid_get_cell(gd,xx,yy,&gc);
   printf(" %u:e%u/%02x%02x%02x%02x/%d %d %d %d %u %u %u ",xx,e->flags,e->data.attr,e->data.fg,e->data.bg,e->data.data,gc.attr,gc.flags,gc.fg,gc.bg,gc.us,gc.link,gc.data.width);
   hex(gc.data.data,gc.data.size);
  }
  if(gl->cellsize)puts("");
 }
}
int main(void) {
 setlocale(LC_CTYPE,"en_US.UTF-8");if(MB_CUR_MAX==1)setlocale(LC_CTYPE,"C.UTF-8");utf8_update_width_cache();
 char line[70000],a[20000],b[20000],*op;struct grid *gd=NULL;struct grid_reader gr;struct screen sc;struct grid_cell last,*lastp=NULL;
 memset(&sc,0,sizeof sc);sc.hyperlinks=hyperlinks_init();memset(&gr,0,sizeof gr);
 while(fgets(line,sizeof line,stdin)) {
  line[strcspn(line,"\n")]=0;op=strtok(line," ");if(!op)continue;
  if(!strcmp(op,"new")){u_int sx=num(),sy=num(),hl=num();if(gd)grid_destroy(gd);gd=grid_create(sx,sy,hl);hyperlinks_reset(sc.hyperlinks);lastp=NULL;gr.gd=gd;}
  else if(!strcmp(op,"clock")){current_time=snum();start_time.tv_sec=snum();}
  else if(!strcmp(op,"set")){u_int px=num(),py=num();struct grid_cell gc=cellspec();grid_set_cell(gd,px,py,&gc);}
  else if(!strcmp(op,"vset")){u_int px=num(),py=num();struct grid_cell gc=cellspec();grid_view_set_cell(gd,px,py,&gc);}
  else if(!strcmp(op,"tab")){u_int px=num(),py=num(),w=num();struct grid_cell gc=cellspec();grid_set_tab(&gc,w);grid_set_cell(gd,px,py,&gc);}
  else if(!strcmp(op,"pad")){u_int px=num(),py=num();int bg=snum();grid_set_padding(gd,px,py,bg);}
  else if(!strcmp(op,"vpad")){u_int px=num(),py=num();int bg=snum();grid_view_set_padding(gd,px,py,bg);}
  else if(!strcmp(op,"cells")){u_int px=num(),py=num();struct grid_cell gc=cellspec();size_t n=unhex(tok(),a);grid_set_cells(gd,px,py,&gc,a,n);}
  else if(!strcmp(op,"vcells")){u_int px=num(),py=num();struct grid_cell gc=cellspec();size_t n=unhex(tok(),a);grid_view_set_cells(gd,px,py,&gc,a,n);}
  else if(!strcmp(op,"clear")){u_int px=num(),py=num(),nx=num(),ny=num();int bg=snum();grid_clear(gd,px,py,nx,ny,bg);}
  else if(!strcmp(op,"clearlines")){u_int py=num(),ny=num();int bg=snum();grid_clear_lines(gd,py,ny,bg);}
  else if(!strcmp(op,"movelines")){u_int dy=num(),py=num(),ny=num();int bg=snum();grid_move_lines(gd,dy,py,ny,bg);}
  else if(!strcmp(op,"movecells")){u_int dx=num(),px=num(),py=num(),nx=num();int bg=snum();grid_move_cells(gd,dx,px,py,nx,bg);}
  else if(!strcmp(op,"scroll")){int bg=snum();grid_scroll_history(gd,bg);}
  else if(!strcmp(op,"scrollregion")){u_int u=num(),l=num();int bg=snum();grid_scroll_history_region(gd,u,l,bg);}
  else if(!strcmp(op,"collect")){grid_collect_history(gd,snum());}
  else if(!strcmp(op,"removehist")){grid_remove_history(gd,num());}
  else if(!strcmp(op,"clearhist")){grid_clear_history(gd);}
  else if(!strcmp(op,"vclearhist")){grid_view_clear_history(gd,snum());}
  else if(!strcmp(op,"vclear")){u_int px=num(),py=num(),nx=num(),ny=num();int bg=snum();grid_view_clear(gd,px,py,nx,ny,bg);}
  else if(!strcmp(op,"vscrollup")){u_int u=num(),l=num();int bg=snum();grid_view_scroll_region_up(gd,u,l,bg);}
  else if(!strcmp(op,"vscrolldown")){u_int u=num(),l=num();int bg=snum();grid_view_scroll_region_down(gd,u,l,bg);}
  else if(!strcmp(op,"vinslines")){u_int py=num(),ny=num();int bg=snum();grid_view_insert_lines(gd,py,ny,bg);}
  else if(!strcmp(op,"vinslinesreg")){u_int rl=num(),py=num(),ny=num();int bg=snum();grid_view_insert_lines_region(gd,rl,py,ny,bg);}
  else if(!strcmp(op,"vdellines")){u_int py=num(),ny=num();int bg=snum();grid_view_delete_lines(gd,py,ny,bg);}
  else if(!strcmp(op,"vdellinesreg")){u_int rl=num(),py=num(),ny=num();int bg=snum();grid_view_delete_lines_region(gd,rl,py,ny,bg);}
  else if(!strcmp(op,"vinscells")){u_int px=num(),py=num(),nx=num();int bg=snum();grid_view_insert_cells(gd,px,py,nx,bg);}
  else if(!strcmp(op,"vdelcells")){u_int px=num(),py=num(),nx=num();int bg=snum();grid_view_delete_cells(gd,px,py,nx,bg);}
  else if(!strcmp(op,"reflow")){grid_reflow(gd,num());}
  else if(!strcmp(op,"setsx")){gd->sx=num();}
  else if(!strcmp(op,"tail")){u_int n=num(),i,total=gd->hsize+gd->sy;grid_adjust_lines(gd,total+n);for(i=0;i<n;i++)grid_empty_line(gd,total+i,8);}
  else if(!strcmp(op,"metadata")){u_int py=num();struct grid_line *gl=&gd->linedata[py];gl->flags=num();gl->time=num();gl->osc133_data.prompt_col=num();gl->osc133_data.cmd_col=num();gl->osc133_data.out_start_col=num();gl->osc133_data.out_end_col=num();gl->osc133_data.exit_status=num();}
  else if(!strcmp(op,"wrappos")){u_int px=num(),py=num(),wx,wy;grid_wrap_position(gd,px,py,&wx,&wy);printf("%u %u\n",wx,wy);}
  else if(!strcmp(op,"unwrappos")){u_int wx=num(),wy=num(),px,py;grid_unwrap_position(gd,&px,&py,wx,wy);printf("%u %u\n",px,py);}
  else if(!strcmp(op,"linelen")){u_int py=num();printf("%u %u\n",grid_line_length(gd,py),grid_line_limit(gd,py));}
  else if(!strcmp(op,"inset")){u_int px=num(),py=num();unhex(tok(),a);printf("%d\n",grid_in_set(gd,px,py,a));}
  else if(!strcmp(op,"link")){unhex(tok(),a);unhex(tok(),b);printf("%u\n",hyperlinks_put(sc.hyperlinks,a,*b?b:NULL));}
  else if(!strcmp(op,"resetlast")){lastp=NULL;}
  else if(!strcmp(op,"string")){u_int px=num(),py=num(),nx=num();int flags=snum(),uselast=snum(),usesc=snum();char *s;
   if(uselast&&lastp==NULL){memcpy(&last,&grid_default_cell,sizeof last);lastp=&last;}
   s=grid_string_cells(gd,px,py,nx,uselast?&lastp:NULL,flags,usesc?&sc:NULL);hex(s,strlen(s));puts("");free(s);}
  else if(!strcmp(op,"vstring")){u_int px=num(),py=num(),nx=num();char *s=grid_view_string_cells(gd,px,py,nx);hex(s,strlen(s));puts("");free(s);}
  else if(!strcmp(op,"dump"))dump(gd);
  else if(!strcmp(op,"rstart")){u_int cx=num(),cy=num();grid_reader_start(&gr,gd,cx,cy);}
  else if(!strcmp(op,"rright")){int w=snum(),al=snum(),om=snum();grid_reader_cursor_right(&gr,w,al,om);}
  else if(!strcmp(op,"rleft")){grid_reader_cursor_left(&gr,snum());}
  else if(!strcmp(op,"rdown")){grid_reader_cursor_down(&gr);}
  else if(!strcmp(op,"rup")){grid_reader_cursor_up(&gr);}
  else if(!strcmp(op,"rsol")){grid_reader_cursor_start_of_line(&gr,snum());}
  else if(!strcmp(op,"reol")){int w=snum(),al=snum();grid_reader_cursor_end_of_line(&gr,w,al);}
  else if(!strcmp(op,"rnextword")){unhex(tok(),a);grid_reader_cursor_next_word(&gr,a);}
  else if(!strcmp(op,"rnextwordend")){unhex(tok(),a);grid_reader_cursor_next_word_end(&gr,a);}
  else if(!strcmp(op,"rprevword")){unhex(tok(),a);int al=snum(),st=snum();grid_reader_cursor_previous_word(&gr,a,al,st);}
  else if(!strcmp(op,"rjump")){struct utf8_data ud;memset(&ud,0,sizeof ud);ud.size=ud.have=unhex(tok(),a);memcpy(ud.data,a,ud.size);ud.width=num();printf("%d\n",grid_reader_cursor_jump(&gr,&ud));}
  else if(!strcmp(op,"rjumpback")){struct utf8_data ud;memset(&ud,0,sizeof ud);ud.size=ud.have=unhex(tok(),a);memcpy(ud.data,a,ud.size);ud.width=num();printf("%d\n",grid_reader_cursor_jump_back(&gr,&ud));}
  else if(!strcmp(op,"rindent")){grid_reader_cursor_back_to_indentation(&gr);}
  else if(!strcmp(op,"rcursor")){u_int cx,cy;grid_reader_get_cursor(&gr,&cx,&cy);printf("%u %u\n",cx,cy);}
  else if(!strcmp(op,"rlinelen")){printf("%u\n",grid_reader_line_length(&gr));}
  else if(!strcmp(op,"rinset")){unhex(tok(),a);printf("%d\n",grid_reader_in_set(&gr,a));}
  else if(!strcmp(op,"names")){int lf=snum(),cf=snum(),at=snum();printf("%s %s %s\n",grid_line_flags_string(lf),grid_cell_flags_string(cf),grid_cell_attr_string(at));}
  else {printf("unknown %s\n",op);}
 }
 return 0;
}
