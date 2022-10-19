#include "margo-logging.h"
#include "mercury_macros.h"
#include <assert.h>
#include <linux/limits.h>
#include <margo.h>
#include <mercury.h>
#include <mercury_core_types.h>
#include <mercury_macros.h>
#include <mercury_proc_string.h>
#include <mercury_types.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void join(hg_handle_t h);
static void set_next(hg_handle_t h);
static void set_prev(hg_handle_t h);
DECLARE_MARGO_RPC_HANDLER(join)
DECLARE_MARGO_RPC_HANDLER(set_next)
DECLARE_MARGO_RPC_HANDLER(set_prev)
static hg_return_t join_ring(hg_string_t _addr);

static struct {
  margo_instance_id mid;
  margo_instance_id client_mid;
  hg_addr_t me;
  hg_addr_t next;
  hg_addr_t prev;
  hg_id_t join_rpc;
  hg_id_t set_next_rpc;
  hg_id_t set_prev_rpc;
} env;

hg_return_t lookup_addr(hg_addr_t *addr, hg_string_t name) {
  hg_return_t ret = margo_addr_lookup(env.mid, name, addr);
  if (ret != HG_SUCCESS) {
    fprintf(stderr, "not found %s (%s)\n", name, HG_Error_to_string(ret));
    // TODO
    return ret;
  }
  return HG_SUCCESS;
}

int main(int argc, char *argv[]) {
  hg_return_t ret;
  char addr_str[PATH_MAX];

  printf("initializing server\n");

  env.mid = margo_init("tcp", MARGO_SERVER_MODE, 1, 10);
  assert(env.mid);

  if ((ret = margo_addr_self(env.mid, &env.me)) != HG_SUCCESS) {
    goto err;
  };
  size_t my_address_size = sizeof(addr_str);
  if ((ret = margo_addr_to_string(env.mid, addr_str, &my_address_size,
                                  env.me)) != HG_SUCCESS) {
    goto err;
  }

  printf("setting rpc handlers\n");
  // TODO: error handling
  // join rpc handler
  env.join_rpc =
      MARGO_REGISTER(env.mid, "join", hg_string_t, hg_string_t, join);
  // set_next rpc handler
  env.set_next_rpc =
      MARGO_REGISTER(env.mid, "set_next", hg_string_t, void, set_next);
  if ((ret = margo_registered_disable_response(env.mid, env.set_next_rpc,
                                               HG_TRUE)) != HG_SUCCESS) {
    goto err;
  }
  // set_prev rpc handler
  env.set_prev_rpc =
      MARGO_REGISTER(env.mid, "set_prev", hg_string_t, void, set_prev);
  if ((ret = margo_registered_disable_response(env.mid, env.set_prev_rpc,
                                               HG_TRUE)) != HG_SUCCESS) {
    goto err;
  }

  printf("initializing address\n");
  // ring init
  // nextとprevを自分自身に向ける
  if ((ret = lookup_addr(&env.next, addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = lookup_addr(&env.prev, addr_str)) != HG_SUCCESS) {
    goto err;
  }

  if (argc > 1) {
    if ((ret = join_ring(argv[1])) != HG_SUCCESS) {
      goto err;
    }
  }

  printf("my addr: %s\n", addr_str);

  margo_wait_for_finalize(env.mid);
  return 0;
err:
  fprintf(stderr, "main: %s\n", HG_Error_to_string(ret));
  return 1;
}

static hg_return_t join_ring(hg_string_t next) {
  hg_return_t ret;

  // self.next = next
  if ((ret = lookup_addr(&env.next, next)) != HG_SUCCESS) {
    return ret;
  }

  hg_handle_t h;
  if ((ret = margo_create(env.mid, env.next, env.join_rpc, &h)) != HG_SUCCESS) {
    return ret;
  }

  char my_addr_str[PATH_MAX];
  size_t my_addr_str_size = sizeof(my_addr_str);
  if ((ret = margo_addr_to_string(env.mid, my_addr_str, &my_addr_str_size,
                                  env.me)) != HG_SUCCESS) {
    goto err;
  }
  printf("my_addr: %s\n", my_addr_str);
  char *my_addr_str_ptr = (char *)my_addr_str;
  if ((ret = margo_forward(h, &my_addr_str_ptr)) != HG_SUCCESS) {
    goto err;
  }

  char prev_addr_str[PATH_MAX];
  if ((ret = margo_get_output(h, prev_addr_str)) != HG_SUCCESS) {
    goto err;
  }

  assert(margo_destroy(h) == HG_SUCCESS);
  // self.prev = next.prev
  return lookup_addr(&env.prev, prev_addr_str);
err:
  margo_destroy(h);
  fprintf(stderr, "join_ring: %s\n", HG_Error_to_string(ret));
  return ret;
}

static void join(hg_handle_t h) {
  hg_return_t ret;
  char *joined_addr_str;
  printf("enter join_rcp handler\n");
  if ((ret = margo_get_input(h, &joined_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  printf("recieved joining address: %s\n", joined_addr_str);
  fflush(stdout);

  // prev.set_next('n)
  hg_handle_t prev_h;
  if ((ret = margo_create(env.mid, env.prev, env.set_next_rpc, &prev_h)) !=
      HG_SUCCESS) {
    goto err_prev;
  }
  if ((ret = margo_forward(prev_h, &joined_addr_str)) != HG_SUCCESS) {
    goto err_prev;
  }
  if ((ret = margo_destroy(prev_h)) != HG_SUCCESS) {
    goto err;
  }

  // return old_prev
  char old_prev[PATH_MAX];
  size_t old_prev_size = sizeof(old_prev);
  if ((ret = margo_addr_to_string(env.mid, old_prev, &old_prev_size,
                                  env.prev)) != HG_SUCCESS) {
    goto err;
  }
  char *old_prev_ptr = (char *)old_prev;
  if ((ret = margo_respond(h, &old_prev_ptr)) != HG_SUCCESS) {
    goto err;
  }

  // self.prev = n'
  if ((ret = lookup_addr(&env.prev, joined_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_free_input(h, &joined_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_destroy(h)) != HG_SUCCESS) {
    goto exit;
  }
  return;
err_prev:
  margo_destroy(prev_h);
err:
  margo_destroy(h);
exit:
  fprintf(stderr, "join_rpc handler: %s\n", HG_Error_to_string(ret));
  exit(1);
}
static void set_prev(hg_handle_t h) {
  hg_return_t ret;
  char *prev_addr_str;
  if ((ret = margo_get_input(h, &prev_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = lookup_addr(&env.prev, prev_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_free_input(h, &prev_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_destroy(h)) != HG_SUCCESS) {
    goto exit;
  }
  return;
err:
  margo_destroy(h);
exit:
  fprintf(stderr, "set_prev_rpc handler: %s\n", HG_Error_to_string(ret));
  exit(1);
}
static void set_next(hg_handle_t h) {
  hg_return_t ret;
  char *next_addr_str;
  if ((ret = margo_get_input(h, &next_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = lookup_addr(&env.next, next_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_free_input(h, &next_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_destroy(h)) != HG_SUCCESS) {
    goto exit;
  }
  return;
err:
  margo_destroy(h);
exit:
  fprintf(stderr, "set_next_rpc handler: %s\n", HG_Error_to_string(ret));
  exit(1);
}

DEFINE_MARGO_RPC_HANDLER(join)
DEFINE_MARGO_RPC_HANDLER(set_prev)
DEFINE_MARGO_RPC_HANDLER(set_next)
