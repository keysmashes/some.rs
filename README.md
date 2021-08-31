# some

some is a meta-pager. If the input is less than one screen long, it will
print it directly to the terminal; otherwise, it will invoke your
regular pager.

some determines which pager to use by inspecting the `$PAGER`
environment variable. As such, it's important that you do not set
`PAGER=some`.
