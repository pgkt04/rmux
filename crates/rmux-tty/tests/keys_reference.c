/* Test driver for unmodified tmux tty-keys.c and key-string.c at 8f25579c. */
#include <locale.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "tmux.h"

/* ---- fake input buffer and timers, substituted before including tty-keys.c */
struct fakebuf { unsigned char data[65536]; size_t len; };
static struct fakebuf inbuf;
static unsigned char *fake_data(struct evbuffer *b) { (void)b; return inbuf.data; }
static size_t fake_len(struct evbuffer *b) { (void)b; return inbuf.len; }
static void fake_drain(struct evbuffer *b, size_t n) {
	(void)b; if (n > inbuf.len) n = inbuf.len;
	memmove(inbuf.data, inbuf.data + n, inbuf.len - n); inbuf.len -= n;
}
struct faketimer { int initialized; int pending; long ms; };
static struct faketimer key_t_, clip_t_;
static struct tty ttyv;
static struct faketimer *ft(struct event *ev) {
	return ev == &ttyv.key_timer ? &key_t_ : &clip_t_;
}
static int fake_initialized(struct event *ev) { return ft(ev)->initialized; }
static int fake_pending(struct event *ev) { return ft(ev)->pending; }
static void fake_del(struct event *ev) { ft(ev)->pending = 0; }
static void fake_set(struct event *ev) { ft(ev)->initialized = 1; }
static void fake_add(struct event *ev, struct timeval *tv) {
	ft(ev)->pending = 1; ft(ev)->ms = tv->tv_sec * 1000 + tv->tv_usec / 1000;
}
#undef EVBUFFER_DATA
#undef EVBUFFER_LENGTH
#undef evbuffer_drain
#undef event_initialized
#undef evtimer_initialized
#undef evtimer_del
#undef evtimer_pending
#undef evtimer_set
#undef evtimer_add
#define EVBUFFER_DATA(b) ((const char *)fake_data(b))
#define EVBUFFER_LENGTH(b) fake_len(b)
#define evbuffer_drain(b, n) fake_drain(b, n)
#define event_initialized(ev) fake_initialized(ev)
#define evtimer_initialized(ev) fake_initialized(ev)
#define evtimer_del(ev) fake_del(ev)
#define evtimer_pending(ev, tv) fake_pending(ev)
#define evtimer_set(ev, cb, arg) fake_set(ev)
#define evtimer_add(ev, tv) fake_add(ev, tv)

#include "tty-keys.c"
#include "key-string.c"

/* ---- stubs */
struct options *global_options = (struct options *)1;
static char *caps[400];
static char *users[KEYC_NUSER + 1];
static long long escape_time = 10;
static int has_requests;
static struct client clientv;
static struct session sessionv;
static struct winlink winlinkv;
static struct window windowv;
static char events[65536];
static void ev(const char *fmt, ...) {
	va_list ap; size_t off = strlen(events);
	va_start(ap, fmt); vsnprintf(events + off, sizeof events - off, fmt, ap); va_end(ap);
}
static void evhex(const char *s, size_t n) {
	size_t off = strlen(events), i;
	for (i = 0; i < n && off + 2 < sizeof events; i++, off += 2)
		snprintf(events + off, 3, "%02x", (unsigned char)s[i]);
}
void log_debug(const char *fmt, ...) { (void)fmt; }
int log_get_level(void) { return 0; }
__dead void fatalx(const char *fmt, ...) { (void)fmt; abort(); }
__dead void fatal(const char *fmt, ...) { (void)fmt; abort(); }
struct options_entry *options_get(struct options *o, const char *n) { (void)o; (void)n; return (struct options_entry *)1; }
struct options_array_item *options_array_first(struct options_entry *o) { (void)o; return NULL; }
struct options_array_item *options_array_next(struct options_array_item *o) { (void)o; return NULL; }
union options_value *options_array_item_value(struct options_array_item *o) { (void)o; return NULL; }
static union options_value uservalue;
union options_value *options_array_getv(struct options_entry *o, const char *fmt, ...) {
	va_list ap; unsigned i; (void)o; (void)fmt;
	va_start(ap, fmt); i = va_arg(ap, unsigned); va_end(ap);
	if (i > KEYC_NUSER || users[i] == NULL) return NULL;
	uservalue.string = users[i]; return &uservalue;
}
long long options_get_number(struct options *o, const char *n) { (void)o; (void)n; return escape_time; }
const char *tty_term_string(struct tty_term *t, enum tty_code_code c) { (void)t; return caps[c] ? caps[c] : ""; }
void tty_set_size(struct tty *tty, u_int sx, u_int sy, u_int xp, u_int yp) {
	tty->sx = sx; tty->sy = sy; tty->xpixel = xp; tty->ypixel = yp; ev(" setsize:%u,%u,%u,%u", sx, sy, xp, yp);
}
void tty_invalidate(struct tty *tty) { (void)tty; ev(" invalidate"); }
void tty_default_features(struct client *c, const char *s, u_int v) { (void)c; (void)v; ev(" deffeat:%s", s); }
/* Features are a set: print them once, in table order, when applied. */
static const char *feat_names[] = { "sixel", "margins", "rectfill", "clipboard", "sync" };
static unsigned feat_mask;
void tty_parse_client_features(struct client *c, const char *s, const char *sep) {
	unsigned i; (void)c; (void)sep;
	for (i = 0; i < 5; i++) if (strcmp(s, feat_names[i]) == 0) feat_mask |= 1u << i;
}
void tty_update_features(struct tty *tty) {
	unsigned i; (void)tty;
	for (i = 0; i < 5; i++) if (feat_mask & (1u << i)) ev(" feat:%s", feat_names[i]);
	feat_mask = 0; ev(" update");
}
void server_client_update_theme_colours(struct client *c) { (void)c; ev(" theme-colours"); }
void session_theme_changed(struct session *s) { (void)s; ev(" theme-changed"); }
void input_request_reply(struct client *c, enum input_request_type t, void *data) {
	(void)c;
	if (t == INPUT_REQUEST_CLIPBOARD) {
		struct input_request_clipboard_data *cd = data;
		ev(" reply-clip:%u:", (unsigned char)cd->clip); evhex(cd->buf, cd->len);
	} else if (t == INPUT_REQUEST_PALETTE) {
		struct input_request_palette_data *pd = data;
		ev(" reply-pal:%d:%d", pd->idx, pd->c);
	}
}
void paste_add(const char *name, char *buf, size_t len) { (void)name; ev(" paste:"); evhex(buf, len); free(buf); }
void window_update_focus(struct window *w) { (void)w; ev(" winfocus"); }
void events_fire_client(const char *name, struct client *c) { (void)c; ev(" fire:%s", name); }
int server_client_handle_key(struct client *c, struct key_event *e) {
	(void)c;
	ev(" key:%llx:", (unsigned long long)e->key); evhex(e->buf, e->len);
	if (e->key == KEYC_MOUSE)
		ev(" mouse:%u,%u,%u,%u,%u,%u,%u,%u", e->m.x, e->m.y, e->m.b, e->m.lx, e->m.ly, e->m.lb, e->m.sgr_type, e->m.sgr_b);
	return 0;
}

/* ---- driver */
static size_t unhex(const char *s, unsigned char *out) {
	size_t n = 0; unsigned v;
	while (s[0] && s[1] && sscanf(s, "%2x", &v) == 1) { out[n++] = v; s += 2; }
	return n;
}
static char *unhexdup(const char *s) {
	unsigned char *out = xmalloc(strlen(s) / 2 + 1); size_t n = unhex(s, out); out[n] = 0; return (char *)out;
}
static void dump(struct tty_key *tk, FILE *f) {
	if (tk == NULL) { fputs("-", f); return; }
	fprintf(f, "(%02x %llx ", (unsigned char)tk->ch, (unsigned long long)tk->key);
	dump(tk->left, f); fputc(' ', f); dump(tk->right, f); fputc(' ', f); dump(tk->next, f); fputc(')', f);
}
static void init_state(void) {
	inbuf.len = 0; memset(&key_t_, 0, sizeof key_t_); memset(&clip_t_, 0, sizeof clip_t_);
	ttyv.flags = 0;
	ttyv.mouse_last_x = ttyv.mouse_last_y = ttyv.mouse_last_b = 0;
	ttyv.sx = 80; ttyv.sy = 24; ttyv.xpixel = ttyv.ypixel = 0; ttyv.fg = ttyv.bg = -1;
	ttyv.tio.c_cc[VERASE] = 0x7f; escape_time = 10; has_requests = 0;
	clientv.session = &sessionv; clientv.flags = 0;
	free(clientv.term_type); clientv.term_type = xstrdup("");
	TAILQ_INIT(&clientv.input_requests);
}
int main(void) {
	char line[200000], *op, *a1, *a2, *a3, *a4;
	setlocale(LC_CTYPE, "en_US.UTF-8"); if (MB_CUR_MAX == 1) setlocale(LC_CTYPE, "C.UTF-8");
	utf8_update_width_cache();
	memset(&ttyv, 0, sizeof ttyv); ttyv.client = &clientv; clientv.name = "c";
	sessionv.curw = &winlinkv; winlinkv.window = &windowv; init_state();
	while (fgets(line, sizeof line, stdin)) {
		line[strcspn(line, "\n")] = 0;
		op = strtok(line, " "); a1 = strtok(NULL, " "); a2 = strtok(NULL, " "); a3 = strtok(NULL, " "); a4 = strtok(NULL, " ");
		if (op == NULL) continue;
		if (strcmp(op, "cap") == 0) { int c = atoi(a1); free(caps[c]); caps[c] = a2 ? unhexdup(a2) : NULL; }
		else if (strcmp(op, "clearcaps") == 0) { int i; for (i = 0; i < 400; i++) { free(caps[i]); caps[i] = NULL; } for (i = 0; i <= KEYC_NUSER; i++) { free(users[i]); users[i] = NULL; } }
		else if (strcmp(op, "user") == 0) { int i = atoi(a1); if (i >= 0 && i <= KEYC_NUSER) { free(users[i]); users[i] = unhexdup(a2 ? a2 : ""); } }
		else if (strcmp(op, "build") == 0) tty_keys_build(&ttyv);
		else if (strcmp(op, "tree") == 0) { dump(ttyv.key_tree, stdout); puts(""); }
		else if (strcmp(op, "init") == 0) init_state();
		else if (strcmp(op, "flags") == 0) ttyv.flags = (ttyv.flags & (TTY_TIMER | TTY_BRACKETPASTE)) | (int)strtoul(a1, NULL, 16);
		else if (strcmp(op, "session") == 0) clientv.session = atoi(a1) ? &sessionv : NULL;
		else if (strcmp(op, "verase") == 0) ttyv.tio.c_cc[VERASE] = atoi(a1);
		else if (strcmp(op, "size") == 0) { ttyv.sx = atoi(a1); ttyv.sy = atoi(a2); ttyv.xpixel = atoi(a3); ttyv.ypixel = atoi(a4); }
		else if (strcmp(op, "colours") == 0) { ttyv.fg = atoi(a1); ttyv.bg = atoi(a2); }
		else if (strcmp(op, "requests") == 0) { clientv.input_requests.tqh_first = atoi(a1) ? (struct input_request *)1 : NULL; }
		else if (strcmp(op, "escape") == 0) escape_time = atoll(a1);
		else if (strcmp(op, "feed") == 0) { inbuf.len += unhex(a1 ? a1 : "", inbuf.data + inbuf.len); }
		else if (strcmp(op, "expire") == 0) key_t_.pending = 0;
		else if (strcmp(op, "step") == 0) {
			int ret; size_t before = inbuf.len;
			events[0] = 0; key_t_.ms = -1;
			ret = tty_keys_next(&ttyv);
			printf("ret=%d consumed=%zu flags=%x timer=%s delay=%ld paste=%d last=%u,%u,%u fg=%d bg=%d size=%u,%u,%u,%u term=",
			    ret, before - inbuf.len, ttyv.flags & ~(TTY_TIMER | TTY_BRACKETPASTE),
			    !(ttyv.flags & TTY_TIMER) ? "idle" : key_t_.pending ? "waiting" : "fired",
			    key_t_.ms, !!(ttyv.flags & TTY_BRACKETPASTE),
			    ttyv.mouse_last_x, ttyv.mouse_last_y, ttyv.mouse_last_b, ttyv.fg, ttyv.bg,
			    ttyv.sx, ttyv.sy, ttyv.xpixel, ttyv.ypixel);
			{ const char *s = clientv.term_type; for (; *s; s++) printf("%02x", (unsigned char)*s); }
			printf(" |%s\n", events);
		}
		else if (strcmp(op, "name") == 0) {
			char *s = unhexdup(a1 ? a1 : ""); key_code k = key_string_lookup_string(s);
			printf("%llx\n", (unsigned long long)k); free(s);
		}
		else if (strcmp(op, "key") == 0) {
			key_code k = strtoull(a1, NULL, 16); const char *s = key_string_lookup_key(k, atoi(a2));
			for (; *s; s++) printf("%02x", (unsigned char)*s); puts("");
		}
		else { printf("bad op %s\n", op); return 1; }
	}
	return 0;
}
