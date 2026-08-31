#include <stdlib.h>
#include <string.h>

#include "mruby.h"

void *toyoterm_mruby_open(void) {
  mrb_state *mrb = mrb_open();
  if (mrb == NULL) {
    return NULL;
  }
  if (mrb->exc != NULL) {
    mrb_close(mrb);
    return NULL;
  }
  return mrb;
}

void toyoterm_mruby_close(void *state) {
  if (state != NULL) {
    mrb_close((mrb_state *)state);
  }
}

static char *copy_mruby_string(mrb_value value) {
  mrb_int length = RSTRING_LEN(value);
  char *copy = malloc((size_t)length + 1);
  if (copy == NULL) {
    return NULL;
  }
  memcpy(copy, RSTRING_PTR(value), (size_t)length);
  copy[length] = '\0';
  return copy;
}

int toyoterm_mruby_eval(void *state, const char *source, char **output) {
  mrb_state *mrb = (mrb_state *)state;
  *output = NULL;
  mrb->exc = NULL;

  mrb_value value = mrb_load_string(mrb, source);
  if (mrb->exc != NULL) {
    mrb_value error = mrb_inspect(mrb, mrb_obj_value(mrb->exc));
    *output = copy_mruby_string(error);
    mrb->exc = NULL;
    return 1;
  }

  value = mrb_obj_as_string(mrb, value);
  if (mrb->exc != NULL) {
    mrb_value error = mrb_inspect(mrb, mrb_obj_value(mrb->exc));
    *output = copy_mruby_string(error);
    mrb->exc = NULL;
    return 1;
  }
  *output = copy_mruby_string(value);
  return *output == NULL ? 2 : 0;
}

void toyoterm_mruby_string_free(char *string) { free(string); }
