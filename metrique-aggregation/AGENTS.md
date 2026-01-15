# Working on the macro
The macro is defined in ../metrique-macro

# Writing Examples
- Examples should always use `metrique` directly for dependencies. DO NOT use `metrique_writer` (note that metrique_writer is re-exported as `metrique::writer`)
- Examples should use a real sink (not TestInspector)
- Examples should include an example of the output to stdout
