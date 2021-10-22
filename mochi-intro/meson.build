project('mochi-intro', 'c',
  version : '0.1',
  default_options : ['warning_level=3'])

margo = dependency('margo')

executable('server',
           'server.c',
           dependencies: margo,
           install : false)
executable('client',
           'client.c',
           dependencies: margo,
           install : false)
