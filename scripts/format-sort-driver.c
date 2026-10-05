/* Reference fixture for tmux sort.c @ 8f25579c. */
#include <sys/types.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <locale.h>
#include "tmux.h"
/* Include unchanged sources to exercise private comparators and paste trees. */
#include "sort.c"
#include "paste.c"

static struct session ss[4];
static struct window ww[3];
static struct winlink ll[4];
static struct window_pane pp[4];
static struct screen screens[4];
static struct layout_cell layouts[4];
static struct paste_buffer bb[4];
static struct client cc[4];
static struct key_binding *kk[4];
static const char *names[] = {"z", "a", "A", "a"};
static const unsigned ids[] = {9, 2, 7, 4};
static const unsigned points[] = {8, 1, 8, 3};
static const unsigned sizes[][2] = {{65536,65536}, {80,24}, {80,10}, {40,48}};
static const int seconds[] = {20, 10, 20, 10};
static const int micros[] = {2, 9, 1, 9};

static void
setup(void)
{
 unsigned i;
 const unsigned pane_window[] = {0, 0, 1, 2};
 const unsigned link_window[] = {0, 1, 0, 2};
 const int indexes[] = {3, -2, 7, 0};
 const char *tables[] = {"root", "prefix", "Prefix", "root"};
 const key_code keys[] = {'a'|KEYC_CTRL, 'c', 'b'|KEYC_META, 'd'};
 global_options = options_create(NULL);
 global_w_options = options_create(NULL);
 for (const struct options_table_entry *oe = options_table;
     oe->name != NULL; oe++) {
  if (oe->scope & OPTIONS_TABLE_WINDOW)
   options_default(global_w_options, oe);
 }
 options_set_number(global_w_options, "pane-base-index", 0);
 RB_INIT(&sessions);
 TAILQ_INIT(&clients);
 for (i = 0; i < 4; i++) {
  ss[i].id = ids[i];
  ss[i].name = (char *)(i == 3 ? "b" : names[i]);
  ss[i].creation_time = (struct timeval){seconds[i], micros[i]};
  ss[i].activity_time = (struct timeval){seconds[3-i], micros[i]};
  RB_INIT(&ss[i].windows);
  RB_INSERT(sessions, &sessions, &ss[i]);
  cc[i].name = names[i];
  cc[i].creation_time = ss[i].creation_time;
  cc[i].activity_time = ss[i].activity_time;
  cc[i].tty.sx = sizes[i][0]; cc[i].tty.sy = sizes[i][1];
  cc[i].flags = i == 1 ? CLIENT_ATTACHED|CLIENT_DEAD :
      i == 2 ? 0 : CLIENT_ATTACHED;
  TAILQ_INSERT_TAIL(&clients, &cc[i], entry);
  bb[i].name = (char *)(i == 3 ? "b" : names[i]);
  bb[i].order = ids[i]; bb[i].size = i == 0 ? 100 : 3;
  RB_INSERT(paste_time_tree, &paste_by_time, &bb[i]);
  key_bindings_add(tables[i], keys[i], NULL, 0, cmd_list_new());
  kk[i] = key_bindings_get(key_bindings_get_table(tables[i], 0), keys[i]);
 }
 for (i = 0; i < 3; i++) {
  ww[i].name = (char *)names[i];
  ww[i].creation_time = ss[i].creation_time;
  ww[i].activity_time = ss[i].activity_time;
  ww[i].sx = sizes[i][0]; ww[i].sy = sizes[i][1];
  ww[i].options = options_create(global_w_options);
  TAILQ_INIT(&ww[i].panes); TAILQ_INIT(&ww[i].z_index);
 }
 for (i = 0; i < 4; i++) {
  pp[i].id = ids[i]; pp[i].active_point = points[i];
  pp[i].sx = sizes[i][0]; pp[i].sy = sizes[i][1];
  pp[i].window = &ww[pane_window[i]];
  pp[i].screen = &screens[i]; screens[i].title = (char *)names[i];
  pp[i].layout_cell = &layouts[i];
  layouts[i].flags = i < 2 ? LAYOUT_CELL_FLOATING : 0;
  TAILQ_INSERT_TAIL(&pp[i].window->panes, &pp[i], entry);
  TAILQ_INSERT_HEAD(&pp[i].window->z_index, &pp[i], zentry);
  ll[i].idx = indexes[i]; ll[i].window = &ww[link_window[i]];
  ll[i].session = &ss[i < 3 ? 0 : 1];
  RB_INSERT(winlinks, &ll[i].session->windows, &ll[i]);
 }
}

static int
indexof(const void *p, unsigned kind)
{
 unsigned i;
 for (i = 0; i < 4; i++) {
  if ((kind == 0 && p == &bb[i]) || (kind == 1 && p == &cc[i]) ||
      (kind == 2 && p == &ss[i]) || (kind == 3 && p == &pp[i]) ||
      (kind == 4 && p == &ll[i]) || (kind == 5 && p == kk[i]))
   return (i);
 }
 abort();
}

int
main(void)
{
 char line[128], op[16];
 unsigned kind, order, reversed, a, b, n, i;
 struct sort_criteria crit;
 void *pa, *pb, **out;
 int result;
 setlocale(LC_CTYPE, "en_US.UTF-8");
 setup();
 while (fgets(line, sizeof line, stdin) != NULL) {
  if (sscanf(line, "%15s %u %u %u %u %u", op, &kind, &order,
      &reversed, &a, &b) < 5) abort();
  crit = (struct sort_criteria){order, reversed, NULL};
  if (strcmp(op, "cmp") == 0) {
   sort_criteria = &crit;
   pa = kind == 0 ? (void *)&bb[a] : kind == 1 ? (void *)&cc[a] :
       kind == 2 ? (void *)&ss[a] : kind == 3 ? (void *)&pp[a] :
       kind == 4 ? (void *)&ll[a] : (void *)kk[a];
   pb = kind == 0 ? (void *)&bb[b] : kind == 1 ? (void *)&cc[b] :
       kind == 2 ? (void *)&ss[b] : kind == 3 ? (void *)&pp[b] :
       kind == 4 ? (void *)&ll[b] : (void *)kk[b];
   result = kind == 0 ? sort_buffer_cmp(&pa,&pb) : kind == 1 ?
       sort_client_cmp(&pa,&pb) : kind == 2 ? sort_session_cmp(&pa,&pb) :
       kind == 3 ? sort_pane_cmp(&pa,&pb) : kind == 4 ?
       sort_winlink_cmp(&pa,&pb) : sort_key_binding_cmp(&pa,&pb);
   printf("%d\n", (result > 0) - (result < 0));
  } else if (strcmp(op, "collect") == 0) {
   out = kind == 0 ? (void **)sort_get_buffers(&n,&crit) :
       kind == 1 ? (void **)sort_get_clients(&n,&crit) :
       kind == 2 ? (void **)sort_get_sessions(&n,&crit) :
       kind == 3 ? (void **)(a == 0 ? sort_get_panes(&n,&crit) :
           a == 1 ? sort_get_panes_session(&ss[0],&n,&crit) :
           sort_get_panes_window(&ww[0],&n,&crit)) :
       kind == 4 ? (void **)(a == 0 ? sort_get_winlinks(&n,&crit) :
           sort_get_winlinks_session(&ss[0],&n,&crit)) :
       (void **)(a == 0 ? sort_get_key_bindings(&n,&crit) :
           sort_get_key_bindings_table(key_bindings_get_table("root",0),&n,&crit));
   for (i = 0; i < n; i++) printf("%s%d", i == 0 ? "" : ",", indexof(out[i],kind));
   printf("\n");
  } else abort();
 }
 return (0);
}
