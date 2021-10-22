project('ring', 'c',
  version : '0.1',
  default_options : ['warning_level=3'])

margo = dependency('margo')

executable('ring',
           'main.c',
		   dependencies : margo,
           install : true)
