/* Ported from tmux json.c and regsub.c @ 8f25579c.
 * Byte arguments and results are hexadecimal; the implementations are linked
 * unmodified from the pinned source tree by format_json_regsub.rs.
 */
#include <sys/types.h>
#include <inttypes.h>
#include <regex.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "tmux.h"

__dead void
fatalx(const char *fmt, ...)
{
	va_list ap;
	va_start(ap, fmt);
	vfprintf(stderr, fmt, ap);
	va_end(ap);
	abort();
}

__dead void
fatal(const char *fmt, ...)
{
	va_list ap;
	va_start(ap, fmt);
	vfprintf(stderr, fmt, ap);
	va_end(ap);
	abort();
}

static char *
unhex(const char *s)
{
	size_t n = strlen(s), i;
	unsigned value;
	char *out;

	if (n % 2 != 0)
		abort();
	out = xcalloc(n / 2 + 1, 1);
	for (i = 0; i < n / 2; i++) {
		if (sscanf(s + 2 * i, "%2x", &value) != 1)
			abort();
		out[i] = value;
	}
	return out;
}

static void
hex(const char *s)
{
	const unsigned char *p = (const unsigned char *)s;
	for (; *p != '\0'; p++)
		printf("%02x", *p);
}

static void
result(const char *status, const char *value)
{
	printf("%s", status);
	if (value != NULL) {
		putchar(' ');
		hex(value);
	}
	putchar('\n');
}

static void
json_case(char **args, int count)
{
	char *input = unhex(args[1]), *cause = NULL, *out = NULL;
	char *key = NULL;
	const char *s = NULL;
	int64_t number = 0;
	int boolean = 0, rc = -1, typed;
	struct json_node *root = json_parse(input, &cause), *node, *value;

	if (root == NULL) {
		result("ERR", cause);
		goto done;
	}
	if (strcmp(args[0], "json") == 0) {
		out = json_to_string(root);
		result("OK", out);
		goto done;
	}
	if (count < 3)
		abort();
	key = unhex(args[2]);
	node = json_find(root, key);
	if (strcmp(args[0], "lookup") == 0) {
		out = json_to_string(node);
		result(out == NULL ? "NONE" : "OK", out);
		goto done;
	}
	if (strcmp(args[0], "array") == 0) {
		if (node == NULL || json_get_array(node, &value) != 0) {
			result("NONE", NULL);
			goto done;
		}
		printf("OK ");
		for (node = json_array_first(value); node != NULL;
		    node = json_array_next(node)) {
			out = json_to_string(node);
			hex(out);
			putchar(',');
			free(out);
			out = NULL;
		}
		putchar('\n');
		if (json_array_first(root) != NULL || json_array_next(root) != NULL ||
		    json_array_next(NULL) != NULL)
			abort();
		goto done;
	}
	if (count != 4)
		abort();
	typed = strcmp(args[0], "find") == 0;
	if (!typed && *key == '\0')
		node = root;
	if (!typed && node == NULL) {
		result("NONE", NULL);
		goto done;
	}
	switch (args[3][0]) {
	case 's':
		rc = typed ? json_find_string(root, key, &s, &cause) :
		    json_get_string(node, &s);
		if (rc == 0)
			out = xstrdup(s);
		break;
	case 'n':
		rc = typed ? json_find_number(root, key, &number, &cause) :
		    json_get_number(node, &number);
		if (rc == 0)
			xasprintf(&out, "%" PRId64, number);
		break;
	case 'b':
		rc = typed ? json_find_boolean(root, key, &boolean, &cause) :
		    json_get_boolean(node, &boolean);
		if (rc == 0)
			xasprintf(&out, "%d", boolean);
		break;
	case 'o':
		rc = typed ? json_find_object(root, key, &value, &cause) :
		    json_get_object(node, &value);
		if (rc == 0)
			out = json_to_string(value);
		break;
	case 'a':
		rc = typed ? json_find_array(root, key, &value, &cause) :
		    json_get_array(node, &value);
		if (rc == 0)
			out = json_to_string(value);
		break;
	default:
		abort();
	}
	result(rc == 0 ? "OK" : "ERR", rc == 0 ? out : cause);
 done:
	free(out);
	free(cause);
	free(key);
	free(input);
	json_destroy_node(root);
}

int
main(void)
{
	char *line = NULL, *cursor, *args[8], *token;
	size_t capacity = 0;
	int count, flags;

	while (getline(&line, &capacity, stdin) != -1) {
		line[strcspn(line, "\n")] = '\0';
		cursor = line;
		count = 0;
		while ((token = strsep(&cursor, " ")) != NULL) {
			if (count == 8)
				abort();
			args[count++] = token;
		}
		if (strcmp(args[0], "regsub") == 0 && count == 5) {
			char *pattern = unhex(args[1]), *with = unhex(args[2]);
			char *text = unhex(args[3]), *out;
			flags = 0;
			if (strchr(args[4], 'e') != NULL)
				flags |= REG_EXTENDED;
			if (strchr(args[4], 'i') != NULL)
				flags |= REG_ICASE;
			if (strchr(args[4], 'n') != NULL)
				flags |= REG_NEWLINE;
			out = regsub(pattern, with, text, flags);
			result(out == NULL ? "ERR" : "OK", out);
			free(pattern);
			free(with);
			free(text);
			free(out);
		} else
			json_case(args, count);
	}
	free(line);
	return 0;
}
