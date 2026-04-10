from setuptools import setup, find_packages

setup(
    name="sct-rucio",
    version="0.1.0",
    description="Rucio transfer tool plugin for sc-transport",
    packages=find_packages(),
    install_requires=["requests>=2.31.0"],
)
