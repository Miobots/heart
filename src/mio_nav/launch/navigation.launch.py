"""AMCL localisation on the saved map, plus Nav2 goals (H2.1).

Run after `sim.launch.py`:
    ros2 launch mio_nav navigation.launch.py
then click "Nav2 Goal" in RViz. This wraps nav2_bringup rather than re-describing its nodes;
everything robot-specific lives in config/nav2_params.yaml.
"""
import os
from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument, IncludeLaunchDescription
from launch.conditions import IfCondition
from launch.launch_description_sources import PythonLaunchDescriptionSource
from launch.substitutions import LaunchConfiguration
from launch_ros.actions import Node

# Sanitize Snap environment contamination if running from VS Code / terminal Snap
if 'LD_LIBRARY_PATH' in os.environ:
    os.environ['LD_LIBRARY_PATH'] = ':'.join([
        p for p in os.environ['LD_LIBRARY_PATH'].split(':')
        if '/snap/' not in p
    ])

for var in ['GTK_PATH', 'GTK_EXE_PREFIX', 'LOCPATH', 'GIO_MODULE_DIR', 'GSETTINGS_SCHEMA_DIR', 'GTK_IM_MODULE_FILE']:
    os.environ.pop(var, None)

if 'XDG_DATA_DIRS' in os.environ:
    os.environ['XDG_DATA_DIRS'] = ':'.join([
        p for p in os.environ['XDG_DATA_DIRS'].split(':')
        if '/snap/' not in p
    ])


def generate_launch_description():
    pkg_mio_nav = get_package_share_directory('mio_nav')
    pkg_nav2_bringup = get_package_share_directory('nav2_bringup')

    map_arg = DeclareLaunchArgument(
        'map',
        default_value=os.path.join(pkg_mio_nav, 'maps', 'home_arena.yaml'),
        description='Full path to the saved map yaml'
    )
    params_arg = DeclareLaunchArgument(
        'params_file',
        default_value=os.path.join(pkg_mio_nav, 'config', 'nav2_params.yaml'),
        description='Full path to the Nav2 parameters file'
    )
    use_sim_time_arg = DeclareLaunchArgument(
        'use_sim_time',
        default_value='true',
        description='Use simulation (Gazebo) clock if true'
    )
    use_rviz_arg = DeclareLaunchArgument(
        'use_rviz',
        default_value='true',
        description='Launch RViz2 with the Nav2 panel if true'
    )

    use_sim_time = LaunchConfiguration('use_sim_time')

    nav2 = IncludeLaunchDescription(
        PythonLaunchDescriptionSource(os.path.join(pkg_nav2_bringup, 'launch', 'bringup_launch.py')),
        launch_arguments={
            'map': LaunchConfiguration('map'),
            'params_file': LaunchConfiguration('params_file'),
            'use_sim_time': use_sim_time,
            'autostart': 'true',
            # No keepout or speed masks exist for this map yet.
            'use_keepout_zones': 'False',
            'use_speed_zones': 'False',
        }.items()
    )

    rviz = Node(
        condition=IfCondition(LaunchConfiguration('use_rviz')),
        package='rviz2',
        executable='rviz2',
        name='rviz2',
        arguments=['-d', os.path.join(pkg_nav2_bringup, 'rviz', 'nav2_default_view.rviz')],
        parameters=[{'use_sim_time': use_sim_time}],
        output='screen',
    )

    return LaunchDescription([
        map_arg,
        params_arg,
        use_sim_time_arg,
        use_rviz_arg,
        nav2,
        rviz,
    ])
